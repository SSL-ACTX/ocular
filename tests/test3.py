import dis
import sys
import time
import ocular

def hot_loop(x):
    total = 0
    for i in range(1_000_000):
        total += x + i
    return total

print("Python version:", sys.version)

# Run once (cold)
start = time.perf_counter()
hot_loop(5)
cold_time = time.perf_counter() - start

ocular.start_tracing(mode="adaptive", deinstrument_threshold=500)

# Run again (should be faster if specialization kicks in)
start = time.perf_counter()
hot_loop(5)
warm_time = time.perf_counter() - start

ocular.stop_tracing()

print(f"Cold run: {cold_time:.6f}s")
print(f"Warm run: {warm_time:.6f}s")

# Inspect bytecode
print("\nDisassembly of hot_loop:")
dis.dis(hot_loop)
