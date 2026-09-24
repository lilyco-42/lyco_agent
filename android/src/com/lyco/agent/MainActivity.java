package com.lyco.agent;

import android.app.Activity;
import android.os.Bundle;
import android.webkit.JavascriptInterface;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import lyco.Lycore;

/**
 * lyco agent —— 本地（离线）CLI 助手的壳。
 *
 * <p>借鉴 {@code lilyco-42/radxa-monitor} 的风格：手写 UI、单文件入口、无 XML layout。
 * 真正的界面在 {@code assets/index.html}，Java 只做两件事：
 * <ol>
 *   <li>把 lycore 的确定性内核暴露成 {@code window.Lycore.*}（WebView JS 桥）</li>
 *   <li>将来的 SSH 执行通道（{@code hw} 命令在 Radxa 板子上，必须用 JSch 远程跑）</li>
 * </ol>
 *
 * <p><b>「伪装的服务端」就是这套东西</b>：页面以为自己在连一个 agent 服务，
 * 实际全部跑在手机本地 —— 不联网、不调云端模型。
 */
public class MainActivity extends Activity {

    private WebView web;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        super.onCreate(savedInstanceState);

        web = new WebView(this);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        // 页面完全来自 assets，不跳外部浏览器
        web.setWebViewClient(new WebViewClient());
        web.addJavascriptInterface(new Bridge(), "Lycore");
        setContentView(web);
        web.loadUrl("file:///android_asset/index.html");
    }

    @Override
    public void onBackPressed() {
        if (web != null && web.canGoBack()) {
            web.goBack();
        } else {
            super.onBackPressed();
        }
    }

    /**
     * {@code window.Lycore.*} —— 每个方法都兜住异常。
     *
     * <p>为什么必须 try/catch：native 层抛异常会直接冒泡到 WebView 之外把
     * Activity 打挂，而这里最常见的失败是"help 文本格式不认识"。
     * 统一返回 {@code {"error": ...}}，让 JS 侧自己判断，比崩掉好。
     */
    final class Bridge {

        @JavascriptInterface
        public String version() {
            try {
                return Lycore.version();
            } catch (Throwable t) {
                return err(t);
            }
        }

        @JavascriptInterface
        public String parseHelp(String cli, String help) {
            try {
                return Lycore.parseHelp(cli, help);
            } catch (Throwable t) {
                return err(t);
            }
        }

        @JavascriptInterface
        public String manual(String cli, String help) {
            try {
                return Lycore.manual(cli, help);
            } catch (Throwable t) {
                return err(t);
            }
        }

        @JavascriptInterface
        public String gate(String cmd) {
            try {
                return Lycore.gate(cmd);
            } catch (Throwable t) {
                return err(t);
            }
        }

        private String err(Throwable t) {
            String m = String.valueOf(t.getMessage());
            // JSON 字符串里不能出现裸引号/换行，否则 JS 的 JSON.parse 直接炸
            m = m.replace("\\", " ").replace("\"", "'").replace("\n", " ");
            return "{\"error\":\"" + m + "\"}";
        }
    }
}
