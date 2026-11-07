# eddy-fetch

通用抓取策略流水线和规则包格式。这个仓库只包含引擎本身和格式说明，不包含任何站点规则。规则包由用户自己维护，放在本机的 `~/Library/Application Support/Eddy/rulepacks/`，或由自己的 eddy-server 下发（见 `docs/adr/0004`）。

## 抓取策略

对一个网址，流水线先直接请求；结果不被接受（例如提取出的正文太短）时，按规则里 `strategies` 的顺序依次尝试：

| 名称 | 作用 |
| --- | --- |
| `headers` | 使用规则里的 `user_agent`、`referer`、`headers` 重新请求 |
| `cookies` | 带上规则里你自己的 Cookie（例如你订阅的站点的登录会话） |
| `archive` | 查询 Internet Archive 是否有存档，取存档里的原始页面 |
| `webview` | 交给平台的 WebView 渲染后取页面（需要应用提供） |

## 规则包格式（TOML）

```toml
name = "我的规则"
version = 1

[[site]]
# 前面带点表示所有子域名
domains = ["example.com", ".example.org"]
strategies = ["headers", "webview"]
user_agent = "Mozilla/5.0 ..."
referer = "https://example.com/"
headers = { "Accept-Language" = "zh-CN" }
cookies = { "session" = "你自己的会话值" }

# 正文提取覆盖：CSS 选择器，或常见的 XPath 写法
body = ["article .post-content"]
strip = [".newsletter-signup", "//aside"]
next_page = "a.next"
```

也可以把 Full-Text RSS 格式（ftr-site-config）的站点文件放进 `rulepacks/ftr/`，文件名是域名，例如 `example.com.txt`。支持的指令：`body`、`strip`、`strip_id_or_class`、`title`、`next_page_link`、`single_page_link`、`http_header(user-agent)`、`http_header(referer)`、`http_header(cookie)`。XPath 只支持能直接换成 CSS 选择器的常见写法，其余条目会被跳过。
