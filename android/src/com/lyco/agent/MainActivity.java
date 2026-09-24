package com.lyco.agent;

import android.app.Activity;
import android.content.Context;
import android.net.nsd.NsdManager;
import android.net.nsd.NsdServiceInfo;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.webkit.JavascriptInterface;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import com.jcraft.jsch.ChannelExec;
import com.jcraft.jsch.JSch;
import com.jcraft.jsch.Session;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicReference;

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

    /** mDNS 服务类型 —— 与 radxa-monitor 一致（板子 avahi 广播的就是这个） */
    private static final String SERVICE_TYPE = "_http._tcp.";

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
        public String rank(String cli, String help, String nl) {
            try {
                return Lycore.rank(cli, help, nl);
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

        /**
         * {@code window.Ssh.discover()} —— mDNS 自动找板子。
         *
         * <p><b>为什么必须自动发现</b>：板子 IP 会漂、还会被别的设备抢占，
         * 让人手填 IP 迟早填错（radxa-monitor 的 {@code nsdDiscover()} 就为这个存在）。
         *
         * <p>返回 {@code [{name, host, port, radxa}]}。
         * <b>返回候选列表而不是只取第一个</b>：局域网里 {@code _http._tcp} 服务往往不止一个
         * （打印机 / NAS / 其它开发板），只取第一个会连到错误的机器上 ——
         * 而"连错机器"正是 IP 漂移事故里最难查的那种错。
         *
         * <p>⚠️ NsdManager 内部用 AsyncChannel，需要 Looper；JS 桥跑在没有 Looper 的
         * JavaBridge 线程上，所以必须 post 到主线程再等待。
         */
        @JavascriptInterface
        public String discover() {
            final NsdManager nsd = (NsdManager) getSystemService(Context.NSD_SERVICE);
            if (nsd == null) {
                return "[]";
            }
            final Handler ui = new Handler(Looper.getMainLooper());
            final CountDownLatch latch = new CountDownLatch(1);
            final List<JSONObject> hits = Collections.synchronizedList(new ArrayList<JSONObject>());
            final AtomicReference<NsdManager.DiscoveryListener> dlRef =
                    new AtomicReference<NsdManager.DiscoveryListener>();

            ui.post(new Runnable() {
                public void run() {
                    NsdManager.DiscoveryListener dl = new NsdManager.DiscoveryListener() {
                        public void onDiscoveryStarted(String t) { }
                        public void onDiscoveryStopped(String t) { }
                        public void onStartDiscoveryFailed(String t, int e) { latch.countDown(); }
                        public void onStopDiscoveryFailed(String t, int e) { }
                        public void onServiceLost(NsdServiceInfo s) { }
                        public void onServiceFound(NsdServiceInfo si) {
                            if (!SERVICE_TYPE.equals(si.getServiceType())) {
                                return;
                            }
                            try {
                                nsd.resolveService(si, new NsdManager.ResolveListener() {
                                    public void onResolveFailed(NsdServiceInfo i, int e) { }
                                    public void onServiceResolved(NsdServiceInfo info) {
                                        if (info.getHost() == null) {
                                            return;
                                        }
                                        String addr = info.getHost().getHostAddress();
                                        if (addr == null) {
                                            return;
                                        }
                                        String name = String.valueOf(info.getServiceName());
                                        // 只是**排序偏好**不是过滤：未知设备也要列出来，
                                        // 万一板子改了名字，用户还能手动认。
                                        boolean isRadxa = name.toLowerCase().contains("radxa");
                                        try {
                                            JSONObject o = new JSONObject();
                                            o.put("name", name);
                                            o.put("host", addr);
                                            o.put("port", info.getPort());
                                            o.put("radxa", isRadxa);
                                            hits.add(o);
                                        } catch (Exception ignored) {
                                        }
                                        // 找到真板子就可以收工；否则等超时兜底
                                        if (isRadxa) {
                                            latch.countDown();
                                        }
                                    }
                                });
                            } catch (Exception ignored) {
                            }
                        }
                    };
                    dlRef.set(dl);
                    try {
                        nsd.discoverServices(SERVICE_TYPE, NsdManager.PROTOCOL_DNS_SD, dl);
                    } catch (Exception e) {
                        latch.countDown();
                    }
                }
            });

            try {
                latch.await(4000, TimeUnit.MILLISECONDS);
            } catch (InterruptedException e) {
                // 超时就返回已经收集到的，不强求
            }

            final NsdManager.DiscoveryListener dl = dlRef.get();
            if (dl != null) {
                ui.post(new Runnable() {
                    public void run() {
                        try {
                            nsd.stopServiceDiscovery(dl);
                        } catch (Exception ignored) {
                        }
                    }
                });
            }

            // 先拷贝再排序：hits 是同步表，直接 sort 可能 ConcurrentModificationException
            List<JSONObject> copy;
            synchronized (hits) {
                copy = new ArrayList<JSONObject>(hits);
            }
            Collections.sort(copy, new java.util.Comparator<JSONObject>() {
                public int compare(JSONObject a, JSONObject b) {
                    boolean ra = a.optBoolean("radxa", false);
                    boolean rb = b.optBoolean("radxa", false);
                    if (ra != rb) {
                        return ra ? -1 : 1;
                    }
                    return a.optString("name").compareTo(b.optString("name"));
                }
            });
            JSONArray arr = new JSONArray();
            for (JSONObject o : copy) {
                arr.put(o);
            }
            return arr.toString();
        }
    }
}
