# test2.py
import time
import os
import ocular

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


def run_one_cycle(enable_tracing: bool, threshold: int = 500) -> tuple[float, int]:
    if enable_tracing:
        ocular.start_tracing(mode="adaptive", deinstrument_threshold=threshold)

    # warmup
    _ = heavy_computation(1000)

    start = time.time()
    result = heavy_computation(20000)
    elapsed = time.time() - start

    if enable_tracing:
        ocular.stop_tracing()

    return elapsed, result


def main():
    print("=== test2.py: tracing overhead comparison ===")

    t_no_trace, res_no_trace = run_one_cycle(False)
    print(f"No tracing:   {t_no_trace:.4f}s (result {res_no_trace})")

    t_trace_fast, res_trace_fast = run_one_cycle(True, threshold=500)
    print(f"With tracing (threshold=500): {t_trace_fast:.4f}s (result {res_trace_fast})")

    t_trace_full, res_trace_full = run_one_cycle(True, threshold=0)
    print(f"With tracing (full/threshold=0): {t_trace_full:.4f}s (result {res_trace_full})")

    print("\n--- Overhead Summary ---")
    overhead_fast = t_trace_fast - t_no_trace
    overhead_pct_fast = (overhead_fast / t_no_trace * 100.0) if t_no_trace > 0 else float('nan')
    print(f"De-instrumented (500) Overhead: {overhead_fast:.4f}s ({overhead_pct_fast:.2f}%)")

    overhead_full = t_trace_full - t_no_trace
    overhead_pct_full = (overhead_full / t_no_trace * 100.0) if t_no_trace > 0 else float('nan')
    print(f"Full Tracing (0) Overhead:      {overhead_full:.4f}s ({overhead_pct_full:.2f}%)")


if __name__ == '__main__':
    main()
