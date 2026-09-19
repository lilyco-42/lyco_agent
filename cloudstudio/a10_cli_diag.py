# 诊断 llama-cli: 有哪些二进制/支持哪些 flag/为什么空输出
import os, subprocess

HOME = os.path.expanduser("~")
BINDIR = os.path.join(HOME, "llama.cpp", "build", "bin")
print("=== build/bin ===", flush=True)
for f in sorted(os.listdir(BINDIR)):
    if os.access(os.path.join(BINDIR, f), os.X_OK):
        print("  ", f, flush=True)

CLI = os.path.join(BINDIR, "llama-cli")
print("\n=== which/version ===", flush=True)
for cmd in (f"{CLI} --version", f"ls -l {CLI}"):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True)
    print(f"$ {cmd}\n  rc={r.returncode} out={(r.stdout or '')[:300]} err={(r.stderr or '')[:300]}", flush=True)

print("\n=== help (grep flags) ===", flush=True)
r = subprocess.run(f"{CLI} --help 2>&1 | grep -E '^\\s+-(-)?(f|p|n|no-cnv|single-turn|temp|ngl|conversation|file|prompt)' ",
                   shell=True, capture_output=True, text=True)
print((r.stdout or "")[:3000], flush=True)
print("--- full help head ---", flush=True)
r2 = subprocess.run(f"{CLI} --help 2>&1 | head -60", shell=True, capture_output=True, text=True)
print((r2.stdout or "")[:3000], flush=True)

# 造一个最小 prompt
open("/tmp/_p.txt", "w", encoding="utf-8").write("<|im_start|>user\nhi<|im_end|>\n<|im_start|>assistant\n")
G = f"{HOME}/router_v4-Q4_K_M.gguf"

print("\n=== try -f (stderr shown) ===", flush=True)
r = subprocess.run(f"{CLI} -m {G} -no-cnv -f /tmp/_p.txt -n 16 --temp 0 -ngl 0",
                   shell=True, capture_output=True, text=True, timeout=600)
print("rc=", r.returncode, flush=True)
print("STDOUT:", (r.stdout or "")[-800:], flush=True)
print("STDERR:", (r.stderr or "")[-1500:], flush=True)

print("\n=== try -p (stderr shown) ===", flush=True)
r = subprocess.run([CLI, "-m", G, "-no-cnv", "-p", "hi", "-n", "16", "--temp", "0", "-ngl", "0"],
                   capture_output=True, text=True, timeout=600)
print("rc=", r.returncode, flush=True)
print("STDOUT:", (r.stdout or "")[-800:], flush=True)
print("STDERR:", (r.stderr or "")[-1500:], flush=True)

print("CLI_DIAG_DONE", flush=True)
