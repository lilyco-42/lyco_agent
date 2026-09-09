#!/usr/bin/env python3
"""vnn_proto.py — lyco_agent VNN (内部识图神经网络) 激活式打分原型

设计 (对齐用户需求):
  OCR 失败 (ocr_pass=False) 才启用 VNN。VNN 不是黑盒分类器, 而是「特征激活」:
    1. 图像 → 轻量 CNN 骨干 (原型用 OpenCV HOG/颜色直方图替代) → 特征向量
    2. 特征向量与「特征神经元库」做相似度 → top-k 激活
       (如: 毛发纹理→激活[动物], 轮廓人脸→激活[人], 窗口边框→激活[GUI])
    3. 激活的语义专家各自打分: 动物专家输出 [猫 0.7 狗 0.2],
       人脸专家输出 [明星A 0.9], GUI 专家输出 [ide 0.6 终端 0.3]
    4. 汇总 → 结构化描述 + 置信度, 低分入学习队列

激活稀疏性: 只 top-k 专家参与计算 (与 lyco_chat block_router 同构),
本地 CPU 可跑, 训练在 CloudStudio。

v0 原型: OpenCV 特征 + 模板匹配式专家。训练原型 (CNN 骨干) 留给 CloudStudio。
"""
import argparse
import json
import math
from pathlib import Path

# ---------- 特征神经元库 (v0: 手工语义标签, 后续由聚类自动生成) ----------
# 每个神经元 = (标签, 特征签名函数名)
FEATURE_NEURONS = ["animal", "human_face", "gui_window", "terminal", "nature", "document"]

def image_features(img_path):
    """OpenCV 轻量特征: 灰度直方图 + 边缘密度 + HOG (原型足够, 免训练)"""
    import cv2
    import numpy as np
    img = cv2.imread(str(img_path))
    if img is None:
        raise FileNotFoundError(img_path)
    gray = cv2.cvtColor(img, cv2.COLOR_BGR2GRAY)
    small = cv2.resize(gray, (64, 64))
    # 颜色/亮度分布 8x8
    hist = cv2.calcHist([small], [0], None, [8], [0, 256]).flatten()
    hist = hist / (hist.sum() + 1e-7)
    # 边缘密度 (Canny 占比) — GUI/文档高, 自然图像低
    edges = cv2.Canny(small, 100, 200)
    edge_density = float((edges > 0).mean())
    # 梯度方向统计 (cv2 5.x headless 无 HOGDescriptor, 用 Sobel 梯度直方图替代)
    gx = cv2.Sobel(small, cv2.CV_32F, 1, 0)
    gy = cv2.Sobel(small, cv2.CV_32F, 0, 1)
    mag = np.sqrt(gx**2 + gy**2).flatten()
    ang = np.arctan2(gy, gx).flatten()
    keep = mag > 1e-3
    hist_ang, _ = np.histogram(ang[keep], bins=16, range=(-np.pi, np.pi),
                               weights=mag[keep])
    hog = hist_ang / (np.linalg.norm(hist_ang) + 1e-7)
    return {"hist": hist, "edge_density": edge_density, "hog": hog,
            "dark_ratio": float((small < 64).mean())}

def activate_neurons(feat, top_k=3):
    """特征 → 激活的特征神经元 (v0: 规则签名, 训练后换成小 MLP)
    实测标定: 终端截图 edge 0.01-0.03, dark 0.99 (极暗底少量文字)
    → 界面类核心特征是 dark_ratio, edge 只做微调"""
    ed, dr = feat["edge_density"], feat["dark_ratio"]
    scores = {}
    scores["terminal"] = dr * min(ed * 30, 1.0)                # 极暗底 + 少量文字
    scores["gui_window"] = (1 - dr) * min(ed * 20, 1.0)        # 亮底 + 边缘
    scores["document"] = (1 - dr) ** 2 * min(ed * 15, 1.0)
    scores["animal"] = (1 - ed * 8) * (0.5 - 0.4 * dr) * 0.6   # 中等亮度有机纹理
    scores["human_face"] = max(0.0, 0.2 - abs(ed - 0.10)) * 2 * (0.3 + 0.4 * (1 - dr))
    scores["nature"] = (1 - ed * 10) * (1 - dr) * 0.8
    ranked = sorted(scores.items(), key=lambda x: -x[1])[:top_k]
    total = sum(s for _, s in ranked) + 1e-7
    return [(k, s / total) for k, s in ranked]

# ---------- 语义专家 (v0: 原型用预置模板; CloudStudio 训练真实专家) ----------
EXPERTS = {
    "terminal": {"desc": "终端/命令行界面", "labels": ["terminal", "ide", "shell"]},
    "gui_window": {"desc": "GUI 窗口界面", "labels": ["app_window", "dialog", "browser"]},
    "document": {"desc": "文本文档", "labels": ["document", "code_page"]},
    "animal": {"desc": "动物", "labels": ["cat", "dog", "bird", "unknown_animal"]},
    "human_face": {"desc": "人脸", "labels": ["known_person", "unknown_person"]},
    "nature": {"desc": "自然风景", "labels": ["sky", "forest", "water"]},
}

def expert_score(neuron, feat, img_path):
    """激活的专家对图像打分。v0: terminal/gui 专家用 OCR 命中词典打分;
    动物/人脸专家输出 unknown_* 并标注 needs_training (诚实)"""
    import numpy as np
    base = {"neuron": neuron, "desc": EXPERTS[neuron]["desc"]}
    if neuron in ("terminal", "gui_window", "document"):
        # 界面类专家: 有文字特征即给结构化分数
        conf = min(0.95, 0.5 + feat["edge_density"])
        base.update({"labels": EXPERTS[neuron]["labels"], "conf": round(conf, 3)})
    else:
        # 动物/人脸专家 v0 未训练: 诚实输出 unknown + needs_training
        base.update({"labels": ["unknown_" + neuron], "conf": 0.0,
                     "needs_training": True})
    return base

def vnn_describe(img_path, ocr_failed=True):
    """VNN 主入口: OCR 失败后调用。返回结构化识图结果"""
    if not ocr_failed:
        return {"skipped": True, "reason": "ocr_passed"}
    feat = image_features(img_path)
    activated = activate_neurons(feat)
    experts = []
    for neuron, weight in activated:
        e = expert_score(neuron, feat, img_path)
        e["activation"] = round(weight, 3)
        experts.append(e)
    trained = [e for e in experts if not e.get("needs_training")]
    if trained:
        best = max(trained, key=lambda e: e["conf"] * e["activation"])
        verdict, conf = best["desc"], best["conf"]
    else:
        verdict, conf = "unknown", 0.0
    return {"image": str(img_path), "verdict": verdict, "conf": conf,
            "activated": experts,
            "learning_queue": [e["neuron"] for e in experts if e.get("needs_training")]}

# ---------- 级联入口 (对接 lyv.py 的 ocr_pass) ----------
def identify(img_path, ocr_text=None, ocr_conf=0.0, min_conf=0.5):
    """VerifierCascade: OCR 优先 → OCR 失败才启用 VNN"""
    from pathlib import Path as P
    ok = bool(ocr_text and ocr_text.strip()) and ocr_conf >= min_conf
    if ok:
        return {"route": "ocr", "text": ocr_text, "conf": ocr_conf}
    result = vnn_describe(img_path, ocr_failed=True)
    result["route"] = "vnn"
    return result

if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("image")
    ap.add_argument("--ocr-text", default="")
    ap.add_argument("--ocr-conf", type=float, default=0.0)
    a = ap.parse_args()
    print(json.dumps(identify(a.image, a.ocr_text, a.ocr_conf), ensure_ascii=False, indent=1))
