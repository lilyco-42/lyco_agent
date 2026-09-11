# -*- coding: utf-8 -*-
"""vnn_train_v9.py — V9: 特征神经元库聚类自动生成 (DESIGN.md 队列项闭环)

升级点 (vs V8):
  1. CNN 主干同 V8 (4 类, 域差距已收敛)
  2. 新增: 用 fc1 32 维特征对每类增强样本做 K-means (k=3/类), 聚出 12 个特征神经元
  3. 神经元库 = {neuron, class, prototype[32], radius} — 数据驱动, 非手写
  4. 导出 v3 格式: 权重 + neurons 库
  5. 验证: 神经元覆盖 (该类样本最近原型属同类) + 真图 9/9
"""
import json, time
import numpy as np
import torch
import torch.nn as nn
import torch.optim as optim

OUT_DIR = "/workspace/vnn_data/model"
CLASSES = ["terminal", "gui_window", "nature", "document"]
K_PER_CLASS = 3
np.random.seed(42)
torch.manual_seed(42)


class TinyCNN(nn.Module):
    def __init__(self, n=4):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.pool = nn.MaxPool2d(2)
        self.fc1 = nn.Linear(16 * 16 * 16, 32)
        self.fc2 = nn.Linear(32, n)

    def forward(self, x):
        x = self.pool(torch.relu(self.conv1(x)))
        x = self.pool(torch.relu(self.conv2(x)))
        x = x.view(-1, 16 * 16 * 16)
        return self.fc2(torch.relu(self.fc1(x)))

    def embed(self, x):
        x = self.pool(torch.relu(self.conv1(x)))
        x = self.pool(torch.relu(self.conv2(x)))
        x = x.view(-1, 16 * 16 * 16)
        return torch.relu(self.fc1(x))


def synth_one(cls):
    if cls == 0:
        img = np.random.uniform(2, 18, (64, 64)).astype(np.float32)
        for _ in range(np.random.randint(2, 8)):
            y = np.random.randint(2, 60)
            if np.random.rand() < 0.4:
                img[0:4, :] = np.random.uniform(90, 150)
                img[4, :] = np.random.uniform(30, 60)
                for _ in range(np.random.randint(1, 4)):
                    yy = np.random.randint(10, 56)
                    xx = np.random.randint(2, 30)
                    img[yy, xx:xx + np.random.randint(4, 16)] = np.random.uniform(80, 190)
            elif np.random.rand() < 0.5:
                x0 = np.random.randint(2, 20)
                x1 = min(62, x0 + np.random.randint(8, 40))
                img[y, x0:x1] = np.random.uniform(120, 255)
                img[y + 1, x0:x1] = np.random.uniform(100, 200) * 0.6
            else:
                img[y, 5:59] = np.random.uniform(100, 220)
    elif cls == 1:
        img = np.random.uniform(180, 240, (64, 64)).astype(np.float32)
        img[5:59, 5:59] = np.random.uniform(150, 220)
        img[8:56, 8:56] = np.random.uniform(200, 250)
    elif cls == 2:
        img = np.random.uniform(80, 180, (64, 64)).astype(np.float32)
        img = np.clip(img + np.random.normal(0, 30, (64, 64)), 0, 255)
    else:
        img = np.full((64, 64), 235.0, dtype=np.float32)
        for y in range(12, 56, 6):
            img[y, 8:56] = np.random.uniform(40, 90)
    img = np.clip(img * np.random.uniform(0.85, 1.15) + np.random.normal(0, 8, (64, 64)), 0, 255)
    return img.astype(np.float32)


def load_real_terminal():
    from PIL import Image
    import glob
    out = []
    for p in sorted(glob.glob("/workspace/vnn_data/real/*.png")):
        im = Image.open(p).convert("L").resize((64, 64))
        out.append(np.asarray(im, dtype=np.float32))
    return out


def gen_batch(bs, device, real_terms, real_p=0.75):
    images, labels = [], []
    for _ in range(bs):
        cls = np.random.randint(0, 4)
        if cls == 0 and real_terms and np.random.rand() < real_p:
            img = real_terms[np.random.randint(0, len(real_terms))].copy()
            cut = np.random.randint(0, 9) if np.random.rand() < 0.5 else 0
            img = np.vstack([img[cut:], np.full((cut, 64), 8.0, dtype=np.float32)])
            img = np.roll(img, np.random.randint(-6, 7), axis=1)
            img = np.clip(img * np.random.uniform(0.9, 1.1) + np.random.normal(0, 6, (64, 64)), 0, 255)
        else:
            img = synth_one(cls)
        images.append(img.astype(np.float32))
        labels.append(cls)
    return torch.tensor(np.array(images)).unsqueeze(1).to(device), torch.tensor(labels).to(device)


def kmeans(X, k, iters=25):
    idx = np.random.choice(len(X), k, replace=False)
    C = X[idx].copy()
    for _ in range(iters):
        d = ((X[:, None, :] - C[None, :, :]) ** 2).sum(-1)
        a = d.argmin(1)
        for j in range(k):
            if (a == j).any():
                C[j] = X[a == j].mean(0)
    d = ((X[:, None, :] - C[None, :, :]) ** 2).sum(-1)
    a = d.argmin(1)
    radii = np.array([np.sqrt(d[a == j, j].max()) if (a == j).any() else 0.0 for j in range(k)])
    return C, radii


def main():
    device = "cuda"
    t0 = time.time()
    real_terms = load_real_terminal()
    print(f"real terminal imgs: {len(real_terms)}", flush=True)

    model = TinyCNN().to(device)
    opt = optim.Adam(model.parameters(), lr=1e-3)
    loss_fn = nn.CrossEntropyLoss()
    model.train()
    for step in range(400):
        x, y = gen_batch(32, device, real_terms)
        loss = loss_fn(model(x), y)
        opt.zero_grad(); loss.backward(); opt.step()
        if step % 100 == 0:
            acc = (model(x).argmax(1) == y).float().mean().item()
            print(f"step {step} loss={loss.item():.4f} acc={acc:.2f} ({time.time()-t0:.0f}s)", flush=True)

    model.eval()
    with torch.no_grad():
        for cls in range(4):
            imgs = np.array([synth_one(cls) for _ in range(200)])
            pred = model(torch.tensor(imgs).unsqueeze(1).to(device)).argmax(1).cpu().numpy()
            print(f"synth {CLASSES[cls]}: {(pred == cls).mean():.0%}", flush=True)
        if real_terms:
            x = torch.tensor(np.array(real_terms)).unsqueeze(1).to(device)
            pred = model(x).argmax(1).cpu().numpy()
            print(f"real terminal: {(pred == 0).sum()}/{len(real_terms)}", flush=True)

    print("clustering neurons...", flush=True)
    neurons = []
    with torch.no_grad():
        for cls in range(4):
            imgs = np.array([synth_one(cls) for _ in range(300)])
            if cls == 0 and real_terms:
                imgs = np.concatenate([imgs, np.array(real_terms)], 0)
            E = model.embed(torch.tensor(imgs).unsqueeze(1).to(device)).cpu().numpy()
            C, radii = kmeans(E, K_PER_CLASS)
            for j in range(K_PER_CLASS):
                neurons.append({
                    "neuron": f"{CLASSES[cls]}_{j}",
                    "class": CLASSES[cls],
                    "prototype": [round(float(v), 5) for v in C[j]],
                    "radius": round(float(radii[j]), 3),
                })
            own_idx = [i for i, n in enumerate(neurons) if n["class"] == CLASSES[cls]]
            allC = np.array([n["prototype"] for n in neurons])
            d = ((E[:, None, :] - allC[None, :, :]) ** 2).sum(-1)
            nearest = d.argmin(1)
            cover = float(np.isin(nearest, own_idx).mean())
            print(f"neuron-cover {CLASSES[cls]}: {cover:.0%}", flush=True)

    weights = {}
    for name, param in model.named_parameters():
        t = param.detach().cpu()
        weights[name] = {"shape": list(t.shape), "data": [round(v, 5) for v in t.flatten().tolist()]}
    json.dump({"classes": CLASSES, "format": "shape+flat", "version": 3,
               "weights": weights, "neurons": neurons},
              open(f"{OUT_DIR}/vnn_cnn_v3_weights.json", "w"), ensure_ascii=False)
    print(f"VNN_TRAIN_V9_DONE neurons={len(neurons)} {time.time()-t0:.0f}s", flush=True)


if __name__ == "__main__":
    main()
