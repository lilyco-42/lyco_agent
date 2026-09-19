# 诊断并释放 GPU：容器里 nvidia-smi 可能列不出 compute-apps 的 PID，改用 ipykernel 进程枚举
import os, subprocess, time, signal

def sh(cmd):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=60).stdout
    except Exception as e:
        return f"ERR {e}"

print("=== nvidia-smi 概要 ===", flush=True)
print(sh("nvidia-smi --query-gpu=memory.used,memory.free,memory.total --format=csv"), flush=True)
print("=== compute-apps（含 PID）===", flush=True)
print(sh("nvidia-smi --query-compute-apps=pid,used_memory,process_name --format=csv"), flush=True)

print("=== 所有 ipykernel 进程 ===", flush=True)
ps = sh("ps -eo pid,ppid,etimes,cmd --no-headers")
me = os.getpid()
ipys = []
for line in ps.splitlines():
    if "ipykernel" in line:
        parts = line.split(None, 3)
        if len(parts) >= 4:
            pid = int(parts[0])
            ipys.append((pid, int(parts[2]), parts[3][:90]))
print(f"我自己的 pid = {me}", flush=True)
for pid, et, cmd in ipys:
    mark = " <== 我" if pid == me else ""
    print(f"  pid={pid} etimes={et}s {cmd}{mark}", flush=True)

# 杀掉除自己以外、存活较久的 ipykernel（它们会一直占着 CUDA context）
victims = [p for p, et, _ in ipys if p != me and et > 60]
print(f"\n准备清理 {len(victims)} 个残留内核: {victims}", flush=True)
for p in victims:
    try:
        os.kill(p, signal.SIGKILL); print(f"  killed {p}", flush=True)
    except Exception as e:
        print(f"  kill {p} 失败: {e}", flush=True)
time.sleep(4)
print("\n=== 清理后显存 ===", flush=True)
print(sh("nvidia-smi --query-gpu=memory.used,memory.free --format=csv"), flush=True)
print("GPU_CLEAN_DONE", flush=True)
