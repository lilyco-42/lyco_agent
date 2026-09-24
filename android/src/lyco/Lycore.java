package lyco;

/**
 * lycore 确定性内核的 JNI 封装 —— 对应 Rust {@code lycore/src/jni_bridge.rs}。
 *
 * <p>类名与包名<b>不能改</b>：JNI 符号是按 {@code Java_lyco_Lycore_xxx} 拼的，
 * 改名会导致 {@code UnsatisfiedLinkError}（这也是 {@code version()} 存在的理由 ——
 * 启动时先调它，能立刻发现名字对没对上，而不是等到"关灯"时才炸）。
 *
 * <p>这里每个方法都是<b>纯确定性、离线、零模型</b>：
 * 解析靠 {@code --help} 文本，翻译靠本地词典，安全判定靠规则表。
 */
public final class Lycore {

    static {
        System.loadLibrary("lycore");
    }

    private Lycore() {
    }

    /** 冒烟：返回 lycore 版本号。起不来就说明 .so 没加载或 JNI 名字不对。 */
    public static native String version();

    /**
     * 解析 {@code cli --help} 原文 → 中文动作表 JSON。
     *
     * <p>字段：{@code full_cmd} / {@code example} / {@code readonly} /
     * {@code desc_en} / {@code desc_zh} / {@code coverage} / {@code unknown}。
     *
     * <p>{@code coverage} 是翻译覆盖率，UI 应把低覆盖条目标出来 —— 译文里
     * 混排的英文就是词典没翻的部分，不许假装翻好了。
     */
    public static native String parseHelp(String cli, String help);

    /** {@code cli --help} 原文 → Markdown 中文说明书。 */
    public static native String manual(String cli, String help);

    /**
     * 执行前的最后一道闸：{@code {risk, decision, reason, param}}。
     *
     * <p>{@code decision}：{@code run}（只读，可直接执行）/
     * {@code confirm}（写操作，需确认）/ {@code block}（破坏性，拦截）/
     * {@code needs_param}（缺参，空跑等于静默失效，不放行）。
     */
    public static native String gate(String cmd);
}
