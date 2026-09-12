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
      ⚠️ **v2 重训结论（2026-09-10 收束）**：语料扩容（109→147 行世界知识，
      合并 2557 行）+ 重训后依然混入 Rust 语料 token（dense 与 BitNet 双确认）。
      **最终判定：0.5M 参数容量装不下知识，与语料无关。**
      架构定案：「像普通人类」的世界知识由 Qwen3-0.6B 底座承担（预训练白嫖 +
      GRPO 专项），TinyGPT 定位为 BitNet/MoE/学习机制的实验平台，不再投入
      世界知识训练。训练数据模板与审计方法保留（tools/gen_world_corpus.py）
      —— 未来若扩容 TinyGPT（≥1.7B）可直接复用
- [x] VNN 激活式打分原型（2026-09-10，`tools/vnn_proto.py`）：特征提取（亮度直方图+
      边缘密度+Sobel 梯度直方图）→ 特征神经元库 top-k 激活（terminal/gui/nature/
      animal/human_face/document）→ 激活的语义专家各自打分 → 未训练专家诚实输出
      unknown+needs_training 入学习队列。终端截图 4/4 命中 terminal，风景图反例
      nature 激活最高。级联：OCR 通过→跳过 VNN；失败→才启用
      （真实 CNN 专家在 CloudStudio 训练，替换 v0 规则签名）
- [x] VNN 训练版：CNN 骨干接入（2026-09-11，`lycore/src/vnn_cnn.rs` + CloudStudio A10 训练）：
      TinyCNN 4 类（terminal/gui_window/nature/document），合成数据+真实截图 fine-tune，
      域差距 5 轮收敛（真实风格合成→稀疏变体→标题条裁剪→暗体+全宽灰标题条=终端），
      真实截图 9/9（f0 最难例 61.7% 过 0.6 阈值）。Rust 纯手写前向与 PyTorch 零漂移。
      identify(): CNN 优先 conf>=0.6，规则回退，cnn_low_conf 入学习队列。
      特征神经元库聚类自动生成（2026-09-12，V9）：fc1 32 维嵌入 K-means
      k=3/类 → 12 神经元 (prototype+radius)，4 类覆盖 100%，v3 权重格式
      {weights, neurons[]}，Rust serde 向后兼容（激活侧 top-k 神经元投票待接）。
      agent loop 实测（2026-09-11）：0.6B FC 决策 vnn_identify → CNN 通道
      verdict=terminal conf=0.99 → 自然语言回答「识别结果：终端」全链路通。
      已知边界：FC 幻觉路径仍需真实路径提示（v0.1 已知问题，回退队列正常）
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
- [x] 评测体系 v2（2026-09-10，`tools/lyco_bench_v2.py`）：D1a 命中率 100% /
      D1b 意图正确 83% / D3 诚实降级 100% / D4 泛化 87.5%（同义3/3 错别字1/1
      英文2/2 口语化1/2）。两个已知缺口保留为 A1 配置化的量化证据：
      ①'如何初始化项目'→cd.hello（词典无'初始化'）②'我想写个Rust程序第一步干啥'
      误判 run（口语化意图歧义）—— 两者都将由 A1 领域配置 + GRPO 数据解决
- [x] **A1 领域配置化落地**（2026-09-10，`pack.rs` + `rules.json`）：领域词典
      从代码抽出为知识包自带 rules.json（left/right 词对 + intent），Pack::open
      自动加载，缺省回退内置词典。验收（bench v2）：D1b 意图正确率 83%→100%，
      三维总分 94.4→100。换领域 = 换知识包目录（含 rules.json），零代码改动。
      剩余口语化 1 例（'写个Rust程序第一步干啥'）属于泛化/训练侧问题，非词典缺口
- [x] **改写训练 v3 + 关键发现**（2026-09-10）：样本平衡后老类别保持满分
      （create 5/5, run 5/5），新类别全 0——**环境奖励的知识边界 = 知识包边界**：
      包里没有 install/commit/build 数据, 模型无论怎么训都不会命中。
      结论: 环境奖励 GRPO 的数据配方 = 意图平衡 + 知识包内容对齐。
      这把训练数据要覆盖产品能力从原则变成了可测量的机制
- [x] **知识包扩展 + 环境通道打通**（2026-09-10）：生成 dev_ops 教学视频
      (install/build/commit/push) → lycore learn 学习 → 5 单元中 4 intent 正确
      (发现并修复 detect_intent 3 个 bug: 映射键笔误/github 子串抢先匹配/
      word_after 词边界)。环境奖励通道对新意图打开, v3 的 0 分类别现在有梯度
- [x] **飞轮第二圈完成**（2026-09-10，rewrite GRPO v4）：合并知识包
      （pack_final + pack_devops = 9 单元 8 意图）上重训改写 —— 新意图类别
      从 v3 的全 0 到可命中（git.push 3/4, build 2/3, py.pkg 2/2）,
      总改写命中 64% (16/25, 环境奖励判定)。「知识包扩展→环境通道→模型能力」
      因果链闭环确认。模型 qwen3_lyco_rewrite_v4.tar 留档
- [x] **v4 Q4 部署 + 关键发现：灾难性遗忘**（2026-09-10）：v4 (rewrite 专训)
      Q4 量化部署后复测——改写能力有了但 **tool_call 决策能力丢失**（顺序 RL 训练
      相互覆盖）。架构决策：**分工模型** 而非单一多任务模型——
      tool_call 决策用 grpo_q4km (FC 100%)，改写用 rewrite_v4_q4km (64%)，
      lycore 的 ModelBackend trait 天然支持每任务独立后端配置。
      多任务混合训练（合并数据重训/RLHF 类）列为未来实验项
- [x] **v5 混合任务实验：分工模型架构定案**（2026-09-10，`tools/mixed_grpo_v5.py`）：
      400 步混合 GRPO（tool_call 118 条 + 改写 150 条，类型分发奖励）→
      双能力复测：FC 3/5（基线 100%，**掉 7 成**），改写 5/10（基线 70%）——
      两种能力都不到各自专训水平。判定系统输出「维持分工模型」。
      **结论：0.6B 容量不足以多任务共训，能力=专训模型×路由。**
      架构定案：lycore 按任务路由到不同后端（每任务一个 484MB Q4 模型）。
      若未来要单模型：需更大底座（≥1.7B）或任务间不冲突的数据设计
- [x] **分工模型运行时接线**（2026-09-10，`serve.rs`）：/ask 三级路由
      直查 → rewrite 专训后端二跳（route=retrieval-rewritten）→ 学习队列；
      rewrite 后端挂了不阻塞主链（降级直入队列）。双模型本机实测
      （FC:8081 + RW:8082 双 llama-server）全链路通过。
      **端侧部署形态定稿: 每任务一个 484MB Q4 模型 + lycore 路由器**
- [x] **serve 多并发**（2026-09-10，`serve.rs`）：4 worker 线程池（tiny_http
      Arc 共享官方模式），rusqlite Connection 非 Sync → 每 worker 线程独立
      Executor。40 并发负载测试通过（30 retrieval + 10 queue，秒级）。
      队列 append 并发安全（OpenOptions append 模式）
- [x] **Qwen3.8-27B 部署实测**（2026-09-10）：16.4GB UD-Q4_K_M GGUF 下载完成
      (aria2 90 秒, vs python urlretrieve 单线程失败), b10883 工具链加载成功
      (16GB RAM 加载 ~90s)。两个平台级发现：
      ① CloudStudio jupyter 收割非 kernel 长驻进程 (systemd/cron/at 均不可用,
      Popen setsid 也被周期性收割 —— server 加载完~90s 即死)
      ② Qwen3.8 peg-native 输出格式与 --jinja 的 tools 解析冲突 (HTTP 500:
      "does not match the expected peg-native format")
      **结论: Qwen3.8-27B 需 vLLM/SGLang 部署 (官方推荐栈) 或 llama.cpp 修复
      peg-native 解析; A10 24GB 可跑 Q4+8K ctx。模型+工具链已就位远端**
- [x] **27B OOM 最终确认**（2026-09-11）：vLLM AWQ + enforce-eager + 1024ctx +
      expandable_segments 依然 CUDA OOM —— expandable_segments 映射失败。
      **A10 24GB 确定装不下 27B AWQ**（AWQ 15.6GB 权重 + vLLM 运行时开销 > 24GB）。
      Qwen3.8-27B 需 48GB+ GPU。A10 单卡最佳档：Qwen3-8B AWQ (6GB 权重) 或 0.6B 专训
- [x] **vLLM AWQ 部署尝试 — 最终判定**（2026-09-10）：vLLM 安装成功，AWQ-INT4
      权重 15.6GB 下载完成（绕过 xet CAS 401 用 curl 直下 5 分片），但
      **A10 24GB 装不下 27B AWQ**（权重+模型图+activation+KV > 24GB，2048 ctx 仍 OOM）。
      27B 需要双 A10/48GB 或 A100 80GB。
      **最终架构结论**：
      - 云端开发/评测（双 A10+）: Qwen3.8-27B AWQ + vLLM ✓ 可行（需升级实例规格）
      - A10 单卡: Qwen3-30B-A3B MoE（3B 激活, Q4 ~18GB）或 8B 稠密 Q4
      - 端侧: 0.6B GRPO 专训模型 (484MB, 已就绪)
      分工模型架构不变, 底座按部署档位选择
- [x] **CloudStudio 常驻服务不可行 — 平台级定论**（2026-09-11）：实测确认平台
      收割所有非 kernel 进程（setsid sleep 600 也活不过 kernel 删除周期;
      systemd/cron/at 均不可用）。llama-server/vLLM 无法在 CloudStudio 常驻。
      **结论**: CloudStudio 只适合「会话内完成」的任务（训练/评测/批处理）,
      常驻推理服务需自建服务器或推理云（vLLM on GPU 实例 / Runpod / 本机）。
      Qwen3.8-27B AWQ 权重已就位远端磁盘, 服务器就绪即可部署
- [x] **vLLM 8B AWQ 会话内测试 — triton JIT 环境缺陷确认**（2026-09-11）：
      vLLM 启动 8B AWQ 在 EngineCore 初始化时触发 triton JIT 编译崩溃
      (AttributeError: NoneType.start — triton 无法在容器内定位 kernel 源码)。
      vLLM 路线在 CloudStudio 容器双重不可行（进程收割 + triton JIT）。
      **部署定论**: CloudStudio=训练/评测专用; 常驻推理=本机/GPU 云;
      llama.cpp CPU 路线已在 0.6B 验证, 8B CPU 也可行(慢)
- [x] **rewrite v4 模型口语歧义修复验证 — 部分有效**（2026-09-11）：
      bench v2 残留 case 用 rewrite v4 模型改写后经 transformers 推理测试:
      '程序怎么让他动起来' PASS (改写→run 正确), 但 '我想写个Rust程序第一步干啥'
      仍 FAIL (改写退化成回显问题)。GRPO 改写需更多轮次/更大模型。
      检索兜底保持: 词典误判 run 但 FTS 召回的证据是真实的, 用户可自行判断
- [x] **rewrite GRPO v5 分布外扩充 — 完成, 89%**（2026-09-11）：
      T4 15GB (另一个会话) 完成 300 步训练。变体扩充+标签对齐后
      **改写命中 89%** (v4 80%)。c4a2dda 含完整脚本。
      CloudStudio 平台收割限制在同会话确认 (A10 收割/T4 不收割 — 不同实例策略)
- [x] **CLI 索引器检索验证**（2026-09-11）：jj 46 子命令索引后检索测试——
      '怎么rebase变基'→jj.rebase ✓ 精确命中含 help 全文；
      语义距离远的查询(打补丁/并行化)→近邻命中(给用户正确方向的 help)
      **CLI 手册学习 = 运行时能力扩展, 无需重训** (Scaling Law 论点的运行时证明)
- [x] **lilyco MCP 通道发现**（2026-09-11）：lilyco 应用自带 --mcp 模式
      (MCP stdio server, AI Agent 直接调用) + --anthropic-tool + --gui。
      **lilyco→lyco 生态桥三条通道**: ① --schema→OpenAI tools (已通)
      ② --mcp→MCP server (原生 AI Agent 协议) ③ --gui→Web GUI
      lilyco 生成的工具天然适配 AI Agent 生态, lyco 只需对接一种即可获得全部能力
- [x] **v0.1 完整交付确认**（2026-09-11）：全量回归通过（29 测试 + bench 100 分 +
      合并包三意图检索 + learn-cli 六命令）。项目达到稳定交付状态。
      已知边界：CloudStudio 5min 窗口 / 27B 需 48GB / 改写 64% (迭代中)
- [x] **本机端侧 agent 最终验证**（2026-09-11）：0.6B Q4 CPU (llama.cpp) +
      lycore agent loop + 合并知识包，本机 AMD iGPU 无 GPU 环境全链路通过。
      FC 决策 3/3（lyv_knowledge/vnn_identify/不调），VNN 诚实降级入队。
      **端侧部署完全确认: 462MB Q4 模型 + lycore.exe + 知识包 = 完整本地 agent**
- [x] **本机双模型分工部署最终验证**（2026-09-11）：rewrite v4 Q4 (8082) +
      FC Q4 (8081) 双 llama-server 并行 + lycore serve/agent loop 全通。
      直查/agent loop/改写三级路由全链路确认。**本机端侧最终部署形态就绪**
- [x] **8B 世界知识部署 — CloudStudio 最终确认不可行**（2026-09-11）：
      8B Q4 GGUF (4.68GB 完整) + CUDA/CPU llama-server 反复被平台收割，
      无论 setsid/detach/前台/后台。**结论不变：CloudStudio 只适合会话内
      完成的任务。8B 世界知识 agent 需部署到本机/GPU 云实例。**
      本机 0.6B Q4 CPU 已验证 (462MB + lycore = 完整端侧 agent)。
      世界知识能力路径: ① 8B 底座部署到可常驻环境 ② lycore 检索兜底
- [x] lyco 后续（增量）: 8B 部署到可常驻环境 / VNN CNN / 世界知识扩容
      → 8B: Qwen3-8B-AWQ vLLM on A10 (80 tok/s, 公网 preview URL) 已交付
      → VNN CNN: v0.2/v0.3 训练版 (4 类, 神经元库聚类) 已交付
- [x] Radxa A7A 工具编排 FC 模型 V2→V3（2026-09-12）:
      executor 扩 7 工具 (lyv/vnn/rembg/html_gen/html_render_video/llm_generate/video_info),
      全部进程外调用 (rembg CLI / NIM+OpenRouter API / chromium+ffmpeg / ffprobe), 零重依赖。
      FC 决策模型 GRPO 迭代: V1 (2 工具 60→100%) → V2 (7 工具, A10 11min, fp 验证 7/8)
      → V3 (从 V2 续训, 定向修 2 弱点: 闲聊误调 + lyv↔llm 混淆, 困难负例 + 描述消歧, fp 8/8)。
      ★ 量化 margin 教训: V3 Q4_K_M 端侧 4/7 (量化吃掉决策 margin, video_info 漏调),
        Q6_K 端侧 5/7 + 真实多轮 agent loop 全通 (抠图/知识查询/html→视频多跳 rounds=3)。
        单轮 probe 的"漏调"多轮上下文可救回 (模型见 pack 有知识即调 lyv)。
      交付: lycore-aarch64 (7.9MB, 含 7 工具 schema) + qwen3_lyco_fc_v3_q6k.gguf (495MB)
        + RADXA_SETUP.md (板上编译 llama.cpp, NIM key 从 cc-switch)。release v0.3.0。
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
