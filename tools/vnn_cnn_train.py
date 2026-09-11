# -*- coding: utf-8 -*-
"""vnn_cnn_train.py — VNN CNN 小型训练 (5min CloudStudio 窗口内)

目标: 训练一个小型 CNN 分类器 (terminal/gui/nature 三类), 替换 vnn_proto 的规则签名。
数据: 程序化生成 (纯色+噪声=terminal暗底, 亮色+纹理=nature, 白底+框线=gui)
模型: 2层Conv + FC, PyTorch, 输入 64x64 灰度 → 3 类
训练: 200 步 (~3min A10), 完成后导出权重 JSON (lycore vnn.rs 可消费)
"""
import json
import torch
import torch.nn as nn
import torch.optim as optim
import numpy as np

OUT = "/workspace/vnn_cnn_weights.json"
CLASSES = ["terminal", "gui_window", "nature"]


class TinyCNN(nn.Module):
    def __init__(self):
        super().__init__()
        self.conv1 = nn.Conv2d(1, 8, 3, padding=1)
        self.conv2 = nn.Conv2d(8, 16, 3, padding=1)
        self.pool = nn.MaxPool2d(2)
        self.fc1 = nn.Linear(16 * 16 * 16, 32)
        self.fc2 = nn.Linear(32, 3)

    def forward(self, x):
        x = self.pool(torch.relu(self.conv1(x)))  # 32x32
        x = self.pool(torch.relu(self.conv2(x)))  # 16x16
        x = x.view(-1, 16 * 16 * 16)
        x = torch.relu(self.fc1(x))
        return self.fc2(x)


def gen_batch(batch_size, device):
    """程序化生成三类 64x64 灰度图"""
    images, labels = [], []
    for _ in range(batch_size):
        cls = np.random.randint(0, 3)
        if cls == 0:  # terminal: 暗底 + 稀疏亮条
            img = np.random.uniform(5, 30, (64, 64)).astype(np.float32)
            for _ in range(np.random.randint(2, 8)):
                y = np.random.randint(2, 60)
                img[y, 5:59] = np.random.uniform(100, 220)
        elif cls == 1:  # gui: 亮底 + 框线
            img = np.random.uniform(180, 240, (64, 64)).astype(np.float32)
            img[5:59, 5:59] = np.random.uniform(150, 220)
            img[8:56, 8:56] = np.random.uniform(200, 250)
        else:  # nature: 中亮度 + 噪声纹理
            img = np.random.uniform(80, 180, (64, 64)).astype(np.float32)
            img = np.clip(img + np.random.normal(0, 30, (64, 64)), 0, 255).astype(np.float32)
        images.append(img)
        labels.append(cls)
    return torch.tensor(np.array(images)).unsqueeze(1).to(device), torch.tensor(labels).to(device)


def main():
    device = "cuda"
    model = TinyCNN().to(device)
    opt = optim.Adam(model.parameters(), lr=1e-3)
    loss_fn = nn.CrossEntropyLoss()

    model.train()
    for step in range(200):
        x, y = gen_batch(32, device)
        pred = model(x)
        loss = loss_fn(pred, y)
        opt.zero_grad()
        loss.backward()
        opt.step()
        if step % 50 == 0:
            acc = (pred.argmax(1) == y).float().mean().item()
            print(f"step {step} loss={loss.item():.4f} acc={acc:.2f}", flush=True)

    # 验证
    model.eval()
    correct = 0
    with torch.no_grad():
        for _ in range(10):
            x, y = gen_batch(32, device)
            pred = model(x).argmax(1)
            correct += (pred == y).sum().item()
    total = 10 * 32
    print(f"验证准确率: {correct}/{total} = {correct/total:.0%}", flush=True)

    # 导出权重 JSON (lycore 可消费)
    weights = {}
    for name, param in model.named_parameters():
        t = param.detach().cpu()
        weights[name] = {"shape": list(t.shape), "data": t.flatten().tolist()}
    json.dump({"classes": CLASSES, "weights": weights},
              open(OUT, "w"), ensure_ascii=False)
    print("VNN_TRAIN_DONE", flush=True)


if __name__ == "__main__":
    main()
