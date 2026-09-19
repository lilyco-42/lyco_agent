# tool_call 契约 + schema 泛化验证（2026-09-19）

## 目的
回答「lyco_agent 以后要调各种 CLI（视频/PS/聊天…）怎么办」：
**模型不背命令，只做「选工具 + 填参数」；工具由 prompt 里的 schema 提供。**

## 契约（读 `lilyco` 源码得到，非猜）
- `tools/list` → `{"tools":[{"name","description","inputSchema"}]}`；非 T0 的 description 追加 ` [safety: T1 confirm]`
- `inputSchema` = `{"type":"object","properties":{...},"required":[...]}`
- `ArgKind → JSON Schema`：Flag=boolean / Text=string / Number=number(+minimum,maximum) / Enum=string+enum / Path=string / List=array+items
- `tools/call` params `{"name","arguments"}`，执行前由 `CommandSchema::validate_args` 校验
- 传输 = 换行分隔 JSON-RPC 2.0 over stdio；模型侧只需把 MCP 形状转成
  **OpenAI 形状**（= `CommandSchema::to_openai_tool()`），**模型不必知道 MCP 细节**

## 实验设置
- 20 个工具参与训练（lffmpeg / lilyco-brush / lilyco-vision / lilyco-chat / 板端 hw 五个「域」）；
  **4 个工具完全留出**：`to_gif`、`upscale`、`skin_retouch`、`describe_image`
- 每个样本的 prompt 里只放 **3 个工具**（模拟"按需挂载 server"），含留出工具时 = 2 已见 + 1 留出
- Qwen3-0.6B，SFT 3 epochs / lr2e-5 / batch4×累积4 / 梯度检查点 / adafactor / bf16 / max_length 1024
- 训练 2940 条；评测：已见 240 题、**留出 32 题**、该不调 11 题

## 结果
| 评测集 | 结果 |
|---|---|
| **S1 已见工具** | **240/240 = 100.0%** |
| **S2 留出工具（仅凭 schema，零训练）** | **26/32 = 81.2%**（失败全是 `wrong_tool`） |
| **S3 该不调（拒绝）** | **11/11 = 100.0%** |

### 结论
1. **「新 CLI 零重训」在 0.6B 上成立**：完全没见过的工具，只要 schema 出现在 prompt 里，
   就能被正确选择并填对参数（81.2%，而 prompt 里 3 选 1 的随机基线约 33%）。
2. 失败模式单一且良性 —— **全是选错工具，没有格式错误、没有参数错误**：
   说明"看 schema 办事"这个能力已经建立，剩下的是描述质量/数据量问题（可继续加训练工具或改描述）。
3. 因此架构上：**lilyco 侧每加一个 CLI 都不用重训模型**，只需让 agent 把新的
   `tools/list` schema 注入 prompt。

## 工程注意（踩过的坑，别再踩）
1. **`tools=` 必须传 OpenAI 形状**（`function.parameters`）；传 MCP 形状（`inputSchema`）模板不认。
2. **工具 schema 让 prompt 暴涨**：6 工具 + `max_length=768` 会把 assistant 目标整个截断 →
   标签全 -100 → 白训。必须放大 `max_length` 并**断言每行监督 token > 0**。
3. **每条工具至少 10+ 道评测题**，否则 N 太小无法判断（最初每工具 1 题 → heldout 只有 4 题，无意义）。
4. **GPU 显存会被历次 ipykernel 残留占满**：容器里 `nvidia-smi --query-compute-apps` 查不到 PID，
   要按 `ps` 枚举 `ipykernel` 并排除自身后 SIGKILL。训练前必须先清理，否则必 OOM。
5. 24G 上跑 0.6B + 长 prompt 的安全配置：**batch4 × 累积4 + 梯度检查点 + max_length 1024**
   （batch8 + max_length1536 会 OOM：抢占到 15.87G 后仍要再要 2.83G）。
