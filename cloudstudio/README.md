# CloudStudio 端侧模型训练管线（Qwen3-0.6B 驾驶模型）

在 CloudStudio GPU 工作空间（A10）上训练 lyco_agent 的**端侧小模型**（"驾驶"模型：工具/技能选择 + 参数填充 + 该不调的判断），并量化为 GGUF `Q4_K_M` 供设备端部署。

设计约束：**本机（Windows）不编译、不训练**。所有环境安装、训练、量化都通过本目录脚本在 CloudStudio 的 Jupyter 内核里执行。

---

## 0. 前置条件

| 项 | 说明 |
|---|---|
| Node | 需要支持顶层 `await` 的版本（本机用 22.x） |
| Python | 工作空间内自带（实测 3.11.1） |
| CloudStudio cookie | 浏览器里取 `cloudstudio-session` + `cloudstudio-session-team=gh` |
| 工作空间状态 | **必须先在控制台手动点「启动」**；API 无启动端点（`/start` 系列全 404） |

> liteApp 数字 ID ≠ spaceKey。`cloudstudio.net/a/<数字>` 里的数字要用
> `GET /api/liteapps/{数字}` 换出真正的 `spaceKey` 才能用。

## 1. 认证：拿 Jupyter Server 地址

```bash
export CS_COOKIE='cloudstudio-session=<值>; cloudstudio-session-team=gh'

# <spaceKey> 换成真实 spaceKey
export CS_JPS=$(node cs_auth.mjs <spaceKey> | head -1 | awk '{print $2}')
```

`cs_auth.mjs` 内部流程：cookie → djb2 CSRF → `/api/user/info` → `/api/workspace/v2/{sk}`
拿 `connections.jupyterServer` → `/api/workspace/{sk}/sessions` 拿 JWT（JWT 仅 5 分钟有效，
每次执行脚本会自行重新 mint，不必手动刷新）。

## 2. 执行器

两者机制相同，只是超时不同。都是把 python 代码丢进 JPS 的 `python3` 内核执行，
并用 subprocess 调本机命令（git / pip / nvidia-smi / 训练脚本）。

```bash
node cs_exec_long.mjs  --file <script.py>   # 超时 60 分钟：pip install 用
node cs_exec_train.mjs --file <script.py>   # 超时 4 小时：训练用
```

> **为什么必须在内核里跑**：CloudStudio 平台会杀掉「非 kernel」的长跑进程。
> 挂后台 / nohup 的训练会被回收；挂在内核执行内的不会。

## 3. 管线顺序

单卡 GPU，请**顺序执行**，避免显存争抢。

```bash
# ① 环境：clone 仓库 + 建 venv + 装 torch
node cs_exec_long.mjs --file a10_stage1.py

# ② 环境：装锁定版本训练栈（含 API 自检）
node cs_exec_long.mjs --file a10_stage2.py

# ③ 训练：工具调用 GRPO → /workspace/qwen3_lyco_grpo
node cs_exec_train.mjs --file a10_train_grpo.py

# ④ 训练：意图→CLI 路由器 SFT → /workspace/qwen3_router_v1
node cs_exec_long.mjs --file a10_train_sft.py

# ⑤ 量化：llama.cpp → GGUF Q4_K_M
node cs_exec_long.mjs --file a10_quantize.py
```

## 4. 为什么锁定这些版本（别随便升级）

工作区间 tensions：`tools/qwen_grpo_train.py`、`tools/fc_grpo_v*.py`、`tools/sft_cli.py`
写于 trl 中期版本，用的是 `GRPOTrainer(reward_funcs=..., processing_class=...)` 签名。
新版 trl 把 `reward_funcs` 改名了，直接装 latest 会让仓库原脚本**静默失效或报错**。
因此 stage2 锁定这套已验证组合：

| 包 | 版本 |
|---|---|
| torch | 2.5.1+cu124 |
| transformers | 4.51.3 |
| trl | 0.16.1 |
| peft | 0.15.2 |
| datasets | 3.3.2 |
| accelerate | 1.6.0 |
| bitsandbytes | 0.45.5 |

stage2 结束会打印 `VERSIONS` 与 `API reward_funcs= True processing_class= True` 作为自检，
两者都 True 才能安全跑后面两步。

## 5. 复用的仓库原脚本（不重复造轮子）

| 仓库脚本 | 作用 | 产出 |
|---|---|---|
| `tools/qwen_grpo_train.py` | 工具调用 GRPO，3 工具，自生成数据，200 步 | `/workspace/qwen3_lyco_grpo` |
| `tools/sft_cli.py` | 意图→CLI 路由器 SFT，吃 `tools/cgidata/*.jsonl` | `/workspace/qwen3_router_v1` |
| `tools/fc_grpo_v4.py` | 扩到 7 工具（**需前驱 final**，缺则 fail-loud） | `/workspace/qwen3_lyco_grpo_v4` |

基座统一为 **`Qwen/Qwen3-0.6B`**（仓库既定端侧基座）。

## 6. 已知坑

- 仓库 trainer 的输出路径**写死 `/workspace/...`**，在容器环境内运行即可，不要改到本机路径。
- `tools/fc_grpo_v4.py` 顶部会 `import vllm`；stage2 **没装 vllm**，所以 v4 不能直接跑（要跑需先装 vllm）。
- GRPO 未开 vLLM（`use_vllm=False`），generation 走 HF `generate`，200 步偏慢，用 4 小时超时那版执行器。
- 方案文档见 `docs/ondevice-training-plan-2026-09-18.md`。
