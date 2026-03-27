# test1.py
import ocular
import time

def primes_sieve(limit):
    is_prime = [True] * (limit + 1)
    p = 2
    while p * p <= limit:
        if is_prime[p]:
            for multiple in range(p * p, limit + 1, p):
                is_prime[multiple] = False
        p += 1
    return [i for i in range(2, limit + 1) if is_prime[i]]


def heavy_computation(n):
    primes = primes_sieve(n)
    total = 0
    for p in primes:
        total += p * p
    for i in range(1, n // 1000 + 1):
        total += i * 12345
    return total


def run_test():
    print("[Test] Starting Ocular Tracer in ADAPTIVE mode (JIT enabled)...")
    ocular.start_tracing(mode="adaptive", perfetto=False)

    print("[Test] Warmup run...")
    _ = heavy_computation(1000)

    print("[Test] Measured run...")
    start_time = time.time()
    result = heavy_computation(20000)
    elapsed = time.time() - start_time

    print(f"[Test] Result: {result} (elapsed {elapsed:.4f}s)")
    print("[Test] Stopping tracer...")
    ocular.stop_tracing()
    print("[Test] Tracer stopped.")


if __name__ == "__main__":
    run_test()
