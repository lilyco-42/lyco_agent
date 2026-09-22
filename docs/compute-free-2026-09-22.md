# 免费算力选型（2026-09-22，CloudStudio 停用后的替代方案）

> 背景：用户「没算力了，自己找免费算力」。CloudStudio 空间当前 `Stopped`（启动须用户在控制台点，且额度受限）；
> 本机**无 GPU**（无 `nvidia-smi`）；板子按新规矩**不跑重负载**。需要的是：**偶尔跑一次 0.6B 全参 SFT + 评测**，
> 不需要 7×24 在线。

## 一、我们要多少算力（实测外推，不是估）

| 项 | 值 | 依据 |
|---|---|---|
| 训练量 | 17,896 条 / **9.30M tokens** | v1 实测 |
| V100-32GB 实测吞吐 | **5,528 tok/s**（bs4×acc4，1.50 s/step，1119 步） | v1 实测 1683 s |
| ⇒ **V100 一次全参 SFT** | **~28 min** | 实测 |
| T4-16GB 推算 | ~**2,200–2,700 tok/s**（T4 fp16 算力 ≈ V100 的 0.4–0.5×） | 外推，待实测 |
| ⇒ **T4 一次全参 SFT** | **~60–70 min** | 外推 |
| 24 题 A/B 评测（0.6B × 2） | 几分钟（GPU）/ CPU 也能跑 | 实测 |

**结论：我们要的是「每小时级、每周几次」的零散 GPU，不是长期占用** —— 完全落在免费额度里。

## 二、候选对比（2026-09 现查，非记忆）

| 平台 | 免费 GPU | 配额 | 会话限制 | 自动化 | 门槛 |
|---|---|---|---|---|---|
| **Kaggle Notebooks** ⭐ | **P100 或 2×T4（16GB）** | **~30 GPU h/周** | 12 h | ✅ **有官方 CLI/API**（`kaggle kernels push/output`） | 需**手机验证**（开 GPU 必需） |
| Lightning AI | T4 / L4 | 15 credits/月（≈22 T4-h） | 无硬限，studio 4h 重启 | ✅ 有 SSH/持久 studio | 需手机验证 |
| Google Colab 免费 | T4（配额**动态**，高峰期常拿不到） | 未公开 | 12 h，**90 min 空闲即断** | ❌ 无 API，需浏览器 | 无 |
| Modal | T4…B300 | $30/月额度 | serverless | ✅ 最好 | ⚠️ **必须绑卡** |
| HF Spaces ZeroGPU | RTX Pro 6000 | **5 min/天** | 每次调用 60s 起 | ✅ | 仅 **推理**，不能训练 |
| AWS SageMaker Studio Lab | T4 | — | 4 h/24h | — | ⛔ **2026-07-30 起停止新注册** |
| Paperspace Gradient 免费 | M4000（8GB） | — | 6 h，且**公开**notebook | ✅ | 8GB 偏小 |

## 三、决策

1. **主：Kaggle**。30 GPU h/周 ≈ 我们**每周 25 次以上**全参 SFT；有 CLI/API ⇒ 可完全脚本化（push → 轮询 → 拉回），
   和以前 CloudStudio 那套工作流一一对应；T4×2 16GB 对 0.6B 全参微调（bs=2）够用。
2. **备：Lightning AI**（15 credits/月，T4 有 SSH 持久 studio）—— 适合 Kaggle 排队时的应急。
3. **应急：Colab 免费**（无 API，只能浏览器手动，本次不做自动化目标）。
4. **评测（CPU 就够）另有一条零成本线**：**GitHub Codespaces**（个人免费 120 core-h/月）。
   你的 `gh` 已登录（lilyco-42），只差一个 scope：`gh auth refresh -h github.com -s codespace`。
   24 题 × 2 模型在 llama.cpp CPU 上约 3 分钟 ⇒ 用来做快速回归很划算。

## 四、已经备好的东西（拿到 token 就能点火）

| 文件 | 作用 |
|---|---|
| `kaggle/_mk_kaggle_v2.py` | 从 v1 训练脚本**派生** Kaggle 版（锚点断言 + 残留校验，命中数不对就 ABORT） |
| `kaggle/train_v2.py`（474 行，语法已验证 ✅） | T4 适配版：`BS=2 ACC=8`、`save_steps=300`、**断点续训**、`HF_HOME=/kaggle/temp` |
| `kaggle/kernel/train_v2.ipynb` + `kernel-metadata.json` | Kaggle kernel 包（`enable_gpu/interenet=true`，private） |

**v2 唯一的科学变量 = 换数据源**：`moss-003（7000 条）→ COIG-CQIA（7000 条）`。
- COIG-CQIA 的 13 个子集字段**统一为 `instruction/input/output`**（用 datasets-server `/info` 探明，不是猜的），
  取 douban/zhihu/xhs/wikihow/segmentfault/coig_pc/human_value/chinese_traditional 八个对话·生活·社区风格子集；
  **刻意排除 exam/logi_qa**（选择题式 QA 会稀释聊天风格）。
- 身份 / JSON / 指令遵循 / 安全拒绝 的定向小样本走 `MICRO=1` 开关，**默认关闭**以保住可归因性
  （想同时上小样本就再跑一臂 v2b，两臂对比）。

## 五、需要你做的最小动作

1. **Kaggle**（必须，约 2 分钟）
   - 注册/登录 → 手机验证（官方要求，否则拿不到 GPU）
   - 右上角头像 → **Settings → API → Create New Token** → 得到 `kaggle.json`
   - 把内容给我（含 `username` 与 `key`），我写到 `~/.kaggle/kaggle.json` 并锁权限
   - 顺带确认一次 **Settings → Phone Verification 已完成**（否则 push 上去也拿不到 GPU）
2. **（可选，评测用）GitHub Codespaces**
   - 你执行一次：`gh auth refresh -h github.com -s codespace`（浏览器确认）
3. 网络：Kaggle / Colab 在国内需走代理 —— 你本机有 Clash，**全局/TUN 模式下即可**（注意别在局域网诊断时开 TUN，见事故档）。

## 六、拿到 token 后我自动执行的序列

```bash
python -m pip install kaggle                       # 或 pipx
# 写入 ~/.kaggle/kaggle.json 并 chmod 600
python kaggle/_mk_kaggle_v2.py --push <你的kaggle用户名>   # 生成 + 推送
kaggle kernels status <user>/lyco-chat-slm-v2             # 轮询
kaggle kernels output  <user>/lyco-chat-slm-v2 -p ./out   # 拉回模型/日志/CHAT_SFT_DONE
```

然后**用同一套 24 题 A/B 做回归**（脚本 `_p2_cs_eval.py` 改两行路径即可），
验收线（沿用 `docs/eval-chat-slm-2026-09-22.md` §四）：
- ✅ 数 **≥ 15**（追平原版底座）
- **G1/G2 必须拒绝**（v1 的删库命令与绕过登录都失守）
- 额外盯：不得再出现「我是 ChatGPT」「写诗重复退化」「改写没改」

## 七、风险与备注

- Kaggle 是**共享 GPU + 排队**，高峰期可能等；`12h/session` 足够（我们需 ~1h），
  但**会话可能被掐** ⇒ 训练脚本已带**断点续训**，`save_steps=300`（约 7 分钟一次检查点）。
- Kaggle `/kaggle/working` 只有 ~20GB 持久空间：模型（~3GB fp32）+ checkpoint 够用；
  HF 数据集缓存已重定向到 `/kaggle/temp`（不占持久空间）。
- 免费额度**随时可能改**（例如 SageMaker Studio Lab 2026-07 直接关停新注册）：本表数字标注了查证日期，
  用前建议再确认一次。
