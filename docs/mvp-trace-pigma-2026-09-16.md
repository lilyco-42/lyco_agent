# MVP：agent trace → pigma 歌词播放（"AI 现场编曲"回放）— 2026-09-16

> 需求（用户原话）：*"ai 现场编曲那种感觉…记录全过程 别人可以播放"*；
> 更早：*"pigma 是用来快速回放别人怎么写出一步步贪吃蛇的，跟随 jj git 记录，每次 commit 一次歌词换行"*。
> 决策：**fork pigma（保留全部 UI 打磨）**，只加一个"内容源 = agent trace"。

## 一、架构（三个部件，各归其位）

```
lycore --trace <f.ndjson>     agent 执行 → 每步一行事件（回放/可视化/打包三用）
        ↓ tail -f
pigma-trace <f.ndjson>        读事件 → 复用 pigma 歌词渲染器（一步=一行歌词）
        ↓ 收尾
mpkg（steps + provenance）     打包成可回放可验证的记忆包        ← 下一步
```

## 二、trace 事件协议（`lycore/src/trace.rs`）

5 种事件，ndjson 一行一条（**每行 flush → `tail -f` 可直播 = "现场"**）：

```json
{"kind":"prompt","i":1,"t_ms":0,"text":"怎么启动 paper 服务器"}
{"kind":"tool","i":2,"t_ms":1165,"name":"shell_exec","arg":"{...}","ok":false}
{"kind":"revert","i":3,"t_ms":1165,"why":"shell_exec 失败: 工作目录不存在: /home/yourdir"}
{"kind":"code","i":4,"t_ms":1500,"file":"src/main.rs","diff":"+fn main()"}
{"kind":"final","i":5,"t_ms":4200,"answer":"完成"}
```

- 对齐 **mpkg spec §6.5 `trace as commits`**（一步 = 一事件 = 一个 jj commit）
- 对齐 **mpkg spec §8 `provenance`**（prompt 谱系与每步溯源 = "编曲注记"）
- **`revert` 是编曲的灵魂**：只记成功步骤 = 成品；记下试错 = 编曲（别人才能听懂为什么改）

## 三、真实验证（A10）

**lycore 侧**（`10dcd92`）：`cargo test` **89 tests → 83 passed / 0 failed**（+2 trace 测试）。
真任务产出 trace（就是最好的演示）：

```json
{"kind":"prompt","i":1,"t_ms":0,"text":"怎么启动 paper 服务器"}
{"kind":"tool","i":2,"t_ms":1165,"name":"shell_exec","arg":"{\"command\":\"paper start\",\"cwd\":\"/home/yourdir\"}","ok":false}
{"kind":"revert","i":3,"t_ms":1165,"why":"shell_exec 失败: sh 启动失败: No such file or directory"}
```
→ **prompt → 工具调用 → 失败 → 回退(带原因)**，闭环成立；顺带暴露并修掉一个真 bug（`cwd` 不存在时 spawn 报误导性 ENOENT）。

**pigma 侧**（fork `7fc2d98`）：
- `src/bin/pigma_trace.rs`：读 ndjson → `LyricLine` → 复用 **`ui::lyrics::draw`**（卡拉OK 渐变原样生效）；
  `--follow`（live tail）/`--speed`；空格暂停 · `j`/`k` 单步 · `q` 退出。
- 改动面极小：`ui.rs` 两处 `pub`、`lyrics.rs` 两处（`draw` 改 pub + `current_song` 变可选，**音乐路径行为不变**）。

## 四、为什么这条路很省（关键发现）

pigma 的歌词渲染**只依赖「歌词行 + 进度」**：
```rust
let dur_secs = match &player.current_song { Some(s)=>…, None => lyrics.last()+3s };
```
所以**不需要新写任何渲染**——造一个 `PlaybackState{ lyrics: 我们的 trace 行, progress: 合成播放头 }` 即可。
另外它的 `translated_lyrics` 是**现成的第二轨槽位**，P1 可直接用作"你的分支轨"（改编/和声）。

## 五、待补（诚实边界）
1. **A10 编译 pigma 尚未完成**（重依赖：rodio/ratatui/ncm-api/sonar/y7dl）——构建中。
2. **`--follow` 直播未实测**（需配合一次真实的边做边跑）。
3. `code` 事件目前**没有生产者**（lycore 尚无"写代码"工具；`file_write` 可接）。
4. 合成播放头是**按时间线性**的；真实回放更适合**按事件序号**（一步一拍）——P1 调整。

## 六、下一步
- **P1-a**：`pigma-trace --follow` 与真实 agent 同步跑一次（真·直播）
- **P1-b**：`mpkg-jj diverge`（分叉后接续）+ pigma 第二轨显示"你的支线"
- **P1-c**：把 trace 收尾打成 mpkg（steps + provenance），走 `cache-node verify`
