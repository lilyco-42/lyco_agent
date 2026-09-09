# lyco_agent — 本地智能协助助手（工具调用 × 技能学习）

> 论点（论文修正版）：**单位算力的系统能力**上，工具、记忆、验证器、技能学习比放大参数更划算；
> 不推翻 Scaling Law，而是改变本地算力的最优分配。依据：Toolformer(2302.04761)、ReAct(2210.03629)、
> VideoAgent(2024, agent 检索关键帧理解长视频)、ToolLoop(2609.09072, 4B+11K 合成数据 → BFCL 86.4%)、
> FEE(2609.08404, 环境反馈 > SFT 预热)、Chinchilla(2203.15556, 算力需联合配置)。

## 架构（接在 lyco_chat 上）

> **2026-09-10 底座决策**（调研: `docs/research-base-model-2026-09-10.md`）：
> 产品级 agent 智商采用混合架构——本地量化 Qwen3-8B 当智商底座（预训练知识白嫖），
> lyco 技能层（tools_openai.json 已注册 lyv_knowledge/vnn_identify）做差异化；
> TinyGPT 保留作 BitNet/MoE 学习实验平台。
> **手机端约束修订 + GRPO 实测**: 用户要求端侧可跑 → 自训路线 R2
> (Qwen3-0.6B 权重继承 → lyco 专项 SFT → MoE 化 → BitNet b1.58 QAT)。
> **GRPO 最小验证已通过: 200 步 RL, FC 遵循度 60%→100%** (A10, 11 分钟)——
> 「工具调用能力靠环境反馈, 不靠模型规模」论点首次实验证实。

```
用户问题 → lyco_chat 9 层路由
             ├─ 命中词典/概念/动作块 → 直接回答
             ├─ 命中技能注册表 SkillRegistry → 调用工具（ocr/asr/vnn/ffmpeg…）
             │    └─ 验证器级联 VerifierCascade（见下）
             ├─ 命中知识包 KnowledgePack(LVK) → 返回 视频切片+帧+文字
             └─ 全落空 → 诚实「不会」→ learned.json / 学习队列
```

### 验证器级联（OCR → 内部识图 NN）
1. **便宜确定性优先**：OCR(tesseract) 提取文字，置信度/关键词命中 → 直接用
2. **OCR 失败信号**：conf < τ 或 关键 token 未命中 → 启用内部识图神经网络打分/描述
3. **仍然低分** → 标记 unknown 入学习队列（诚实，不兜底编造）

### 技能即工具（SkillRegistry）
每个技能 = toolcall schema（lyco_chat 已支持 `tools_openai.json`）+ 成功率统计（FEE 式：
按历史 ok 率选路，失败自动降级到下一技能）。新增技能零训练——只需 schema + 可执行体。

## LVK 格式 v0.1（lyco video knowledge，新视频知识格式）

动机：GUI 教学知识里，视频 > 图片/文本（有精确位置的操作演示 + 时间轴）。
一句话一个知识单元，锚定「时间切片 + 帧 + 双源文字」：

```jsonc
{
  "video": "rust_tutorial.mp4", "lang": "zh",
  "units": [{
    "id": "u003", "t0": 41.2, "t1": 46.8,
    "clip": "clips/u003.mp4",          // ffmpeg 切出的语句级片段
    "text": "首先 cargo new hello_world 建立项目",   // ASR/SRT 文字
    "frame": "frames/u003_f1.jpg",     // 与文字对齐的最佳帧
    "ocr": "PS> cargo new hello_world",// 帧上 OCR
    "ocr_conf": 0.91,
    "strong": ["cargo", "new", "hello_world"],  // ASR∩OCR 双源命中 = 强关联
    "weak": ["建立", "项目"],                     // 单源 = 弱关联
    "verify": { "ocr_pass": true }               // 级联验证结果
  }]
}
```

- **强关联**：ASR 文字 ∩ 帧 OCR 文字都出现的术语（跨模态互证）→ 检索主键
- **弱关联**：单源出现 → 次级召回
- 用户问「怎么新建 rust 项目」→ 强关联命中 `cargo new` → 返回 u003 片段 + 帧 + OCR 验证过的文字解释

## 知识生产管线（学习过程）

```
视频 → ffmpeg 提音频 → faster-whisper ASR → SRT 语句切分
     → ffmpeg 逐句切片 + 抽帧 → tesseract OCR
     → 双源对齐(强/弱关联) → LVK 知识包
     → (可选) 生成 toolcall 语料 → lyco_chat 再训练（ToolLoop 式闭环）
```

## 状态

- [x] 管线原型 `tools/lyv.py`（ffmpeg+tesseract，双源对齐，build/query/cut）
- [x] 冒烟验证（2026-09-10，SRT 路径）：intent 词典命中 → OCR grounding VERIFY_PASS；
      FTS 兜底、NO_HIT 入学习队列、按需切片全部通过
- [x] ASR 远端路径端到端验证（2026-09-10，CloudStudio A10）：edge-tts 神经语音合成
      视频 → whisper-small@CUDA 远端转写（四句全对，时间戳与场景对齐）→ 词典/FTS
      检索 → grounding VERIFY_PASS → 按需切片 8s 片段。A10 上 large-v3 转写 20s 音频
      仅 1.3s，small 为默认档
- [x] 商业重定位决策（CBAM）：B2B 企业视频知识库，见
      `.agents/results/architecture/cbam-lyco-agent-repositioning.md`
      （领域配置化/来源授权元数据/计算策略路由为后续投资序列）
- [x] 训练/编译通道（2026-09-10，CloudStudio A10）：`/workspace/lyco_chat` 源码同步 +
      `cargo build --release --features cuda` 成功（1m53s），`candle device: cuda(0)`
      GPU 推理冒烟通过。远端 CUDA 构建环境：nvcc 12.2 + gcc/g++-12（NVCC_CCBIN）+
      PATH 加 `/root/.cargo/bin:/usr/local/cuda/bin`
- [x] 世界知识语料原型（2026-09-10）：`tools/gen_world_corpus.py`（L1 常识/L2 生活推理/
      L3 自我认知三层，答案语义一致性审计）→ 合并 2519 行语料 → A10 上 15000 步
      candle CUDA 训练完成，`model/world_base.json`（6.9MB）已回传本地。
      ⚠️ 已知问题：embed64/4层 + block16 容量不足，生成仍混入 Rust 语料 token——
      需扩容模型 + 域标签条件生成（Rust 实现阶段处理）
- [x] VNN 激活式打分原型（2026-09-10，`tools/vnn_proto.py`）：特征提取（亮度直方图+
      边缘密度+Sobel 梯度直方图）→ 特征神经元库 top-k 激活（terminal/gui/nature/
      animal/human_face/document）→ 激活的语义专家各自打分 → 未训练专家诚实输出
      unknown+needs_training 入学习队列。终端截图 4/4 命中 terminal，风景图反例
      nature 激活最高。级联：OCR 通过→跳过 VNN；失败→才启用
      （真实 CNN 专家在 CloudStudio 训练，替换 v0 规则签名）
- [ ] VNN 训练版：CNN 骨干 + 特征神经元库由聚类自动生成（CloudStudio）
- [x] Agent 多轮工具执行闭环（2026-09-10，`tools/lyco_agent_loop.py`）：
      模型 tool_call → 真实执行（lyv sqlite 检索 / VNN）→ 结果回填 → 多轮 ≤4 →
      最终回答。A10 实测三场景：HIT（回答带切片时间戳+关键帧）、NO_HIT
      （诚实"还没学会，已入学习队列"）、闲聊（不调工具直接答）。
      工程细节：模型幻觉 pack 路径→回退默认包；复读 JSON→转自然语言
- [x] 长任务多跳 GRPO 第二轮（2026-09-10，`tools/lyco_multihop_train.py`）：
      新增 check_prereq 工具 + prereq 追查奖励（首跳+1/追查+2），从第一轮
      GRPO 产物继续训 250 步（2.2s/步）。实测任务链：查运行知识 → check_prereq
      → 追查 create → 再验证 —— 4 跳链路学会（奖励 shaping 有效）
      模型留档 models/qwen3_lyco_multihop.tar
- [x] **lycore v0.1**（2026-09-10，`lycore/`，Rust 实现库启动）：第一个原子单元
      `pack::Pack::lookup` —— LVK 知识包 sqlite/FTS5 检索（intent 词典→FTS 兜底），
      与 Python `lyv.lookup` 结果一致性集成测试通过（intent/t0/t1/retrieval 全对齐）。
      单测 3 + 集成 2 全绿。发现并修复：规则匹配须按规则独立判定（共享 Matcher
      会让第一条规则吃掉所有命中）
- [x] lycore 原子单元 2: seg_zh tokenizer（2026-09-10，`src/tokens.rs`）——
      与 Python `lyv.tokens()` 逐字节对齐（含跨空格 bigram " 建"/"先 " 等微妙语义，
      findall 词边界语义）。金标准法：token_ref.py 运行时输出 → JSON → Rust 全量
      比对测试，9 用例零漂移。此对齐是双实现 FTS 召回一致性的根基
- [x] lycore 原子单元 3: tool executor + 学习队列（2026-09-10，`src/executor.rs`）——
      tool_call 宽松解析（<tool_call> 包裹/裸 JSON 两级）→ 真实执行（lyv_knowledge 走
      pack::lookup）→ ToolResult JSON；NO_HIT/VNN 未实现 → learning_queue.jsonl 落盘
      （诚实降级，不编造）。集成测试 3 个：HIT 证据返回/NO_HIT 入队/未知工具诚实失败。
      移植细节：Rust f64 Display "15" vs Python f-string "15.0" —— clip 格式须 {:.1}
- [x] lycore 原子单元 4: verifier cascade（2026-09-10，`src/verify.rs`）——
      OCR 量化判定（conf>=min_conf + 精确/lev<=2 模糊命中）+ 级联路由
      （OCR pass→ocr / 失败→learning_queue, vnn_hint 降级提示）。OCR 本体走
      进程外 tesseract（零绑定依赖, LYV_TESSERACT 可覆盖路径, TSV conf index 10
      + tessedit_create_tsv=1 的踩坑结论已固化在代码里）。真实帧集成测试 2 个：
      PASS 路径 + 降级路径全绿
- [x] lycore 原子单元 5: agent loop 编排（2026-09-10，`src/agent.rs`）——
      `ModelBackend` trait 抽象模型 IO（ScriptedBackend 回放测试 / 未来
      LlamaCppBackend、BitNetBackend 可插）+ `Agent::run` 多轮编排
      （tool_call→执行→回填→收束，≤max_rounds 守卫）。三个集成测试用今天
      GRPO 模型在 A10 的**真实输出**做脚本回放：HIT 收束 / NO_HIT 人话转译+
      学习队列 / 轮数守卫。清理 dead code（旧 Matcher）
- [x] lycore 原子单元 6: lernen 学习回流（2026-09-10，`src/lernen.rs`）——
      learning_queue.jsonl → digest（去重+频次统计+reason 分类）→ training_tasks.jsonl
      （GRPO 奖励环境可直接消费的 prompt/expected_tool 对，频次降序=优先级）。
      坏行跳过不阻塞。测试：去重计数/分类/JSONL 输出
- [x] lycore 原子单元 7: LlamaCppBackend（2026-09-10，`src/llamacpp.rs`）——
      ModelBackend 的第一个真实实现：llama.cpp server OpenAI 兼容接口
      (/v1/chat/completions + /health)。tool_calls 字段自动拼回 <tool_call> 文本
      供 parse_call；Qwen3 enable_thinking=false 经 chat_template_kwargs 传递；
      模板由 server 端 (--jinja) 应用，避免双端模板漂移。reqwest blocking + json
- [x] lycore 原子单元 8: CLI 二进制（2026-09-10，`src/bin/lycore.rs`）——
      `ask`（有 llama-server 走 agent loop / 无则纯检索，NO_HIT 自动入队）/
      `harvest`（学习队列→training_tasks.jsonl）/ `doctor`（包/队列/服务体检）。
      端到端实测：HIT 返回切片证据、NO_HIT 入队、harvest 回流 1 任务。
      部署物：lycore 二进制 + llama-server + GGUF 模型 + 知识包目录
- [x] lycore 原子单元 9: learn 端侧知识包生产（2026-09-10，`src/learn.rs`）——
      SRT 驱动：解析（CRLF 容忍）→ ffmpeg 抽帧 → tesseract OCR → 强/弱关联挖掘
      → intent 检测（SUBCMDS 上下文纠错 no→new 同 Python）→ sqlite/FTS 索引
      （schema 与 Python build 一致）。CLI `lycore learn` 端到端实测：4/4 单元
      intent 正确，自学的包 ask 检索全命中。**端侧自主学习链路打通**
- [x] lycore 原子单元 10: serve 常驻模式（2026-09-10，`src/serve.rs`）——
      HTTP API: POST /ask（llama 有→agent loop / 无→纯检索+队列）、GET /health、
      GET /queue、POST /harvest。tiny_http 单线程，端侧足够。全端点实测通过，
      NO_HIT 自动入队 + harvest 采集 2 任务闭环。
      ⚠️ Git Bash curl 发 UTF-8 body 会坏（用 Python client/PowerShell 测试）
- [x] **GGUF 部署链打通**（2026-09-10，CloudStudio）：GRPO 模型 (transformers)
      → convert_hf_to_gguf.py → F16 GGUF → llama-server (--jinja, OpenAI 兼容)
      → HTTP agent loop 真实执行 lyv 检索并收束回答。模型输出:
      "[1] CALL lyv_knowledge{query:怎么新建rust项目} → [1] RESULT 切片3.0-10.0s
      → [2] FINAL 好的!您可以在3.0到10.0秒之间创建项目" ——
      训练产物→生产部署的最后一公里完成, 部署栈 = llama-server + lycore + 知识包
- [x] **Q4 量化版**（2026-09-10）：llama-quantize Q4_K_M，1.5GB→**484MB**（÷3.1），
      llama-server 加载后 agent 全链路复测通过（CALL→RESULT→FINAL）。
      Q4 模型已回传本地 `models/qwen3_lyco_grpo_q4km.gguf` —— 手机端形态达标
      （0.6B Q4 ≈ 484MB + llama.cpp Android 可跑）
- [x] lycore 原子单元 11: 端侧 ASR 学习路径（2026-09-10，`learn::asr_local`）——
      ffmpeg 提 16kHz wav → whisper.cpp (base 模型 141MB, LYV_WHISPER_BIN/MODEL
      可覆盖) → Cue → build_cues。`lycore learn --video` 无 SRT 即走此路径。
      端到端实测：无字幕视频 → 3 单元 intent 全对（含 word_after ASCII 截断修复）
      → ask 全命中。**"给视频就学会"完全端侧化, 与 SRT 路径共用 build_cues 核心**
- [x] lycore 原子单元 12: VNN Rust 版（2026-09-10，`src/vnn.rs`）——
      ffmpeg rawvideo 管道取 64x64 灰度（无 OpenCV）→ edge/dark/Sobel 方向直方图
      特征 → top-k 神经元激活（标定同 Python）→ 专家打分。executor 的
      vnn_identify 从"诚实降级"升级为"真实打分"，失败仍诚实入队。
      真实帧测试：terminal 激活 0.88、verdict"终端/命令行界面"。
      **端侧级联 OCR→VNN 双实现（Py+Rust）全部就位**
- [x] **本地全栈部署验证**（2026-09-10，Windows）：Q4 GGUF (484MB) + llama.cpp
      Win-x64 + lycore CLI 全本机跑通 agent loop（ask --llama），2 轮收束带证据。
      修复 LlamaCppBackend 双重 tool_call 序列化（--jinja content 已含文本时不再拼接；
      arguments 字符串先解析为 JSON 消除 unicode 双重转义）。
      **端侧形态定稿: lycore.exe + llama-server.exe + 484MB GGUF + 知识包目录**
- [ ] lycore 后续: τ²-Bench 评测 / 多并发 / VNN CNN 训练版（CloudStudio）
- [ ] SkillRegistry 接入 lyco_chat 路由（tools_openai.json 扩展）

## 已知经验（fixture 教训）补充

- **lyco_chat 语料是编译期嵌入**（`include_str!`）：换 corpus.json 必须重编二进制，
  且分词必须走 `seg_zh` 单字切分（词级语料会让 `seg_tokens` 生成的查询 token 找不到词表项 panic）
- **Jupyter contents API 下载陷阱**：`format=text` 时 content 是文本本身，
  `format=base64` 才是 base64——cs_download 无脑 base64 解码，对 .json 文本文件会产出
  二进制垃圾；下载文本文件用 `?content=1` 直接写字符串，或先 tar 成二进制再传

## 已知经验（fixture 教训）

- **PowerShell 5.1 按 ANSI 读 UTF-8 无 BOM 脚本**：中文经 .ps1 传给 edge-tts 会念成
  mojibake——中文参数走 bash 或 UTF-8 BOM 文件
- **本地 ffmpeg 解码 edge-tts VBR mp3 有变速损坏**（4.0s→3.58s）：faster-whisper 内部
  PyAV 解码无损，跨机传输音频时上传原始 mp3、在远端解码混音，勿本地重采样
- **Jupyter contents API 路径相对 /workspace**：`lyv_tmp/x` 而非 `/workspace/lyv_tmp/x`，
  且父目录须先建
- **Windows 上 node 退出偶发 libuv 断言**（0xC0000409）：cs_* 工具链成败看 stdout，
  不能看 returncode
- **CS JWT 有效期 5 分钟**：长任务中途须重新 mint token
