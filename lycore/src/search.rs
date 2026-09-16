//! search — agent 自主检索 (「懂得自己去搜」) + 检索结果学习进知识包
//!
//! 补上用户指令链的断点: *"懂得自己去搜 paper minecraft 知道学习怎么启动"*。
//! lyco 原有 7 工具均无检索能力 → 本模块提供:
//!
//!   query → SearchBackend(搜索) → SearchResult[] → 转 Cue → 进 LVK 知识包
//!                                                      → 此后 lyv_knowledge 可答
//!
//! 后端可插换 (`SearchBackend` trait):
//!   - `SearxngBackend`: 自建 SearXNG (`{base}/search?q=&format=json`), 隐私可控 (lyco 卖点)
//!   - `StubSearch`: 离线确定性占位 (单测 / 无网环境), 不触网
//!
//! **隐私取舍 (诚实记录)**: lyco 主打本地隐私, 故搜索**不进 CHAT_TOOLS 常驻工具集**,
//! 而是显式调用的摄取动作 (`lycore search`), 默认离线 (StubSearch)。要成为模型可自主
//! 调用的第 8 个技能, 需同时改 capability/skill/CHAT_TOOLS 三处单一真源并**重训 FC 模型**
//! (现模型为 7 工具契约), 见 `docs/research-self-search-2026-09-16.md`。

use crate::learn::Cue;
use std::path::Path;

/// 一条搜索结果
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// 搜索错误
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchError {
    Http(String),
    Parse(String),
    Io(String),
}

impl std::fmt::Display for SearchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SearchError::Http(e) => write!(f, "搜索 HTTP 错误: {e}"),
            SearchError::Parse(e) => write!(f, "搜索响应解析失败: {e}"),
            SearchError::Io(e) => write!(f, "摄取入库失败: {e}"),
        }
    }
}

impl std::error::Error for SearchError {}

/// 搜索后端抽象
pub trait SearchBackend {
    fn search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>, SearchError>;
}

/// 自建 SearXNG 后端 (`GET {base}/search?q=...&format=json`)
pub struct SearxngBackend {
    base: String,
}

impl SearxngBackend {
    pub fn new(base: &str) -> Self {
        Self {
            base: base.trim_end_matches('/').to_string(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }
}

impl SearchBackend for SearxngBackend {
    fn search(&self, query: &str, k: usize) -> Result<Vec<SearchResult>, SearchError> {
        crate::tools_runtime::install_crypto_provider();
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| SearchError::Http(e.to_string()))?;
        // 注意: reqwest 的 **blocking** RequestBuilder 无 .query() (仅 async 有) → 手工编码
        let url = format!("{}/search?q={}&format=json", self.base, urlencode(query));
        let resp = client
            .get(url)
            .header("User-Agent", "lyco-agent/0.1")
            .send()
            .map_err(|e| SearchError::Http(e.to_string()))?;
        let body = resp.text().map_err(|e| SearchError::Http(e.to_string()))?;
        parse_searxng(&body, k)
    }
}

/// 最小 percent-encoding (UTF-8 逐字节; 空格 %20)。blocking RequestBuilder 无 .query(),
/// 故手工拼 query string。
fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push_str("%20"),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// 解析 SearXNG JSON (`{"results":[{"title","url","content"}]}`)
pub fn parse_searxng(body: &str, k: usize) -> Result<Vec<SearchResult>, SearchError> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|e| SearchError::Parse(e.to_string()))?;
    let arr = v["results"]
        .as_array()
        .ok_or_else(|| SearchError::Parse("响应缺 results 数组".into()))?;
    Ok(arr
        .iter()
        .filter_map(|r| {
            let url = r["url"].as_str().unwrap_or("").trim().to_string();
            if url.is_empty() {
                return None;
            }
            Some(SearchResult {
                title: r["title"].as_str().unwrap_or("").trim().to_string(),
                url,
                snippet: r["content"].as_str().unwrap_or("").trim().to_string(),
            })
        })
        .take(k)
        .collect())
}

/// 离线占位后端 (确定性, 不触网) —— 单测与无网环境用
pub struct StubSearch {
    results: Vec<SearchResult>,
}

impl StubSearch {
    pub fn new(results: Vec<SearchResult>) -> Self {
        Self { results }
    }

    /// 演示用固定结果 (含 paper/minecraft 领域内容)
    pub fn demo(query: &str) -> Self {
        Self {
            results: vec![
                SearchResult {
                    title: format!("{query} — PaperMC 官方文档"),
                    url: "https://docs.papermc.io/".to_string(),
                    snippet: "Paper 服务端启动: java -Xmx2G -jar paper.jar nogui; 首次须 eula=true".to_string(),
                },
                SearchResult {
                    title: format!("{query} — Minecraft Wiki: Server"),
                    url: "https://minecraft.wiki/w/Server".to_string(),
                    snippet: "服务端 jar 须匹配 Java 版本 (1.20.5+ 用 Java 21); 内存按在线人数调整".to_string(),
                },
            ],
        }
    }
}

impl SearchBackend for StubSearch {
    fn search(&self, _query: &str, k: usize) -> Result<Vec<SearchResult>, SearchError> {
        Ok(self.results.iter().take(k).cloned().collect())
    }
}

/// 搜索结果 → 知识 Cue (含来源 URL, 便于溯源/验证)
pub fn to_cues(query: &str, results: &[SearchResult]) -> Vec<Cue> {
    results
        .iter()
        .map(|r| Cue {
            t0: 0.0,
            t1: 0.0,
            text: format!("{query} : {} : {} (来源: {})", r.title, r.snippet, r.url),
        })
        .collect()
}

/// 端到端: 搜索 → 转 Cue → 写入知识包 (幂等, prefix="search")。返回入库条数。
pub fn search_and_learn<B: SearchBackend>(
    pack_dir: &Path,
    query: &str,
    backend: &B,
    k: usize,
) -> Result<usize, SearchError> {
    let results = backend.search(query, k)?;
    let cues = to_cues(query, &results);
    if cues.is_empty() {
        return Ok(0);
    }
    crate::learn_cli::append_cues(pack_dir, "search", &cues).map_err(|e| SearchError::Io(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_searxng_extracts_and_limits() {
        let body = r#"{"results":[
            {"title":" Paper 文档 ","url":"https://docs.papermc.io/","content":" 启动 java -jar paper.jar "},
            {"title":"Minecraft Wiki","url":"https://minecraft.wiki/","content":"server"},
            {"title":"空 url","url":"","content":"应被丢弃"}
        ]}"#;
        let r = parse_searxng(body, 5).unwrap();
        assert_eq!(r.len(), 2, "空 url 应丢弃");
        assert_eq!(r[0].title, "Paper 文档", "应 trim");
        assert_eq!(r[0].url, "https://docs.papermc.io/");
        assert!(r[0].snippet.contains("paper.jar"));
        // k 限制
        assert_eq!(parse_searxng(body, 1).unwrap().len(), 1);
    }

    #[test]
    fn parse_searxng_rejects_bad_json() {
        assert!(matches!(parse_searxng("not json", 3), Err(SearchError::Parse(_))));
        assert!(matches!(parse_searxng("{}", 3), Err(SearchError::Parse(_))));
    }

    #[test]
    fn urlencode_handles_cjk_and_spaces() {
        assert_eq!(urlencode("paper mc"), "paper%20mc");
        assert_eq!(urlencode("a-b_c.d~e"), "a-b_c.d~e");
        // CJK 逐字节 UTF-8 百分号编码 ("中" = E4 B8 AD)
        assert_eq!(urlencode("中"), "%E4%B8%AD");
        assert!(!urlencode("怎么启动 paper").contains(' '));
    }

    #[test]
    fn stub_search_respects_k() {
        let s = StubSearch::demo("paper");
        assert_eq!(s.search("paper", 1).unwrap().len(), 1);
        assert_eq!(s.search("paper", 10).unwrap().len(), 2);
    }

    #[test]
    fn to_cues_carries_source_url() {
        let s = StubSearch::demo("paper minecraft");
        let results = s.search("paper minecraft", 5).unwrap();
        let cues = to_cues("paper minecraft", &results);
        assert_eq!(cues.len(), results.len());
        assert!(cues[0].text.contains("paper minecraft"), "prefix 为 query");
        assert!(cues[0].text.contains("来源:"), "须带来源 URL 便于溯源");
        assert!(cues[0].text.contains("papermc.io"));
    }

    #[test]
    fn search_and_learn_writes_pack() {
        let t = tempfile::tempdir().unwrap();
        let n = search_and_learn(t.path(), "paper minecraft 怎么启动", &StubSearch::demo("x"), 5).unwrap();
        assert_eq!(n, 2);
        // 入库后 FTS 表可查 (segments 有 search.* 条目)
        let db = rusqlite::Connection::open(t.path().join("index/knowledge.sqlite")).unwrap();
        let cnt: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM segments WHERE intent LIKE 'search.%'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cnt, 2, "应向知识包写入 2 条 search.* 条目");
    }

    #[test]
    fn search_and_learn_idempotent() {
        let t = tempfile::tempdir().unwrap();
        let s = StubSearch::demo("x");
        search_and_learn(t.path(), "q", &s, 5).unwrap();
        search_and_learn(t.path(), "q", &s, 5).unwrap(); // 重复 → 幂等, 不翻倍
        let db = rusqlite::Connection::open(t.path().join("index/knowledge.sqlite")).unwrap();
        let cnt: i64 = db
            .query_row("SELECT COUNT(*) FROM segments WHERE intent LIKE 'search.%'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(cnt, 2, "幂等: 重复摄取不累积");
    }
}
