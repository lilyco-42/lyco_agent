package com.lyco.agent;

import android.app.Activity;
import android.os.Bundle;
import android.webkit.JavascriptInterface;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import com.jcraft.jsch.ChannelExec;
import com.jcraft.jsch.JSch;
import com.jcraft.jsch.Session;

import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;

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
        web.addJavascriptInterface(new Ssh(), "Ssh");
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

    /**
     * {@code window.Ssh.exec(host, user, pass, cmd)} —— 在板子上真正跑命令。
     *
     * <p><b>这是「关灯」闭环的最后一环</b>：{@code hw} 是 Radxa 板子上的程序，
     * 命令必须在板子上执行。手机只负责理解与展示，不假装自己是执行者。
     * 通道直接借鉴 {@code radxa-monitor} 的 {@code runSsh()}（JSch ChannelExec）。
     *
     * <p>返回 {@code {exit, out, err}}。{@code exit} 是<b>真实退出码</b> ——
     * 它比"命令发出去了"重要得多：{@code npm install} 空跑也是 0，
     * 只有退出码 + 输出能判断到底干没干成。
     */
    final class Ssh {

        /**
         * @param sudo 是否用 sudo 提权（`hw led blue off` 要写 sysfs，必须 root）
         *
         * <p><b>刻意不改 sudoers</b>：改 {@code /etc/sudoers.d/} 属于系统配置，
         * 那是需要显式确认的事。这里走 {@code sudo -S} 从 <b>stdin</b> 喂密码 ——
         * 密码不会出现在命令行里（{@code ps} 看不到），效果相同但零系统改动。
         */
        @JavascriptInterface
        public String exec(String host, String user, String pass, String cmd, boolean sudo) {
            Session session = null;
            ChannelExec ch = null;
            try {
                JSch jsch = new JSch();
                session = jsch.getSession(user, host, 22);
                session.setPassword(pass);
                session.setConfig("StrictHostKeyChecking", "no");
                session.connect(8000);

                ch = (ChannelExec) session.openChannel("exec");
                // `-p ''`：不要 sudo 的密码提示符，免得混进 stdout
                ch.setCommand(sudo ? ("sudo -S -p '' " + cmd) : cmd);
                if (sudo) {
                    ch.setInputStream(
                        new ByteArrayInputStream((pass + "\n").getBytes("UTF-8")));
                }
                ByteArrayOutputStream errs = new ByteArrayOutputStream();
                ch.setErrStream(errs);
                InputStream in = ch.getInputStream();
                ch.connect(10000);

                ByteArrayOutputStream outs = new ByteArrayOutputStream();
                byte[] buf = new byte[4096];
                int n;
                while ((n = in.read(buf)) > 0) {
                    outs.write(buf, 0, n);
                }
                // ⚠️ 必须等通道关闭再取退出码，否则恒为 -1（radxa-monitor 同款坑）
                while (!ch.isClosed()) {
                    try {
                        Thread.sleep(50);
                    } catch (InterruptedException e) {
                        break;
                    }
                }
                int code = ch.getExitStatus();

                JSONObject o = new JSONObject();
                o.put("exit", code);
                o.put("out", outs.toString("UTF-8"));
                o.put("err", errs.toString("UTF-8"));
                return o.toString();
            } catch (Throwable t) {
                try {
                    JSONObject o = new JSONObject();
                    o.put("exit", -1);
                    o.put("err", String.valueOf(t.getMessage()));
                    return o.toString();
                } catch (Exception e2) {
                    return "{\"exit\":-1,\"err\":\"json fail\"}";
                }
            } finally {
                if (ch != null) {
                    try {
                        ch.disconnect();
                    } catch (Exception ignored) {
                    }
                }
                if (session != null) {
                    try {
                        session.disconnect();
                    } catch (Exception ignored) {
                    }
                }
            }
        }
    }
}
