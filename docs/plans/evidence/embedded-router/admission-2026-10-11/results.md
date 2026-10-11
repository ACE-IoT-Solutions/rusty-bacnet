# Same-executable repeatability observations

Both columns measure the same executable, not a baseline and optimized
candidate. Values are the Criterion `mean.point_estimate` and its 95%
bootstrap confidence interval in nanoseconds per call. These means differ
slightly from the regression slopes printed as `time:` in console output.
Delta is `(B / A - 1) × 100`; it represents repeatability noise here.
Workload semantics and limitations are in [README.md](README.md).

| Workload | A mean [95% CI], ns | B mean [95% CI], ns | B vs A |
|---|---:|---:|---:|
| broadcast/cache_128/fanout_8 | 135.70 [134.94, 136.39] | 136.89 [136.17, 137.62] | +0.9% |
| broadcast/cache_32/fanout_4 | 53.57 [53.19, 53.94] | 52.69 [52.30, 53.05] | -1.6% |
| broadcast/cache_8/fanout_2 | 34.45 [34.23, 34.68] | 34.38 [34.17, 34.61] | -0.2% |
| duplicate/cache_128/hit_last | 99.06 [97.62, 100.61] | 104.83 [102.90, 107.09] | +5.8% |
| duplicate/cache_128/miss_replace | 145.72 [143.35, 148.49] | 147.78 [145.85, 149.78] | +1.4% |
| duplicate/cache_32/hit_last | 26.29 [26.00, 26.64] | 26.08 [25.90, 26.25] | -0.8% |
| duplicate/cache_32/miss_replace | 33.44 [33.10, 33.82] | 33.10 [32.81, 33.48] | -1.0% |
| duplicate/cache_8/hit_last | 8.15 [7.95, 8.34] | 8.23 [8.06, 8.40] | +0.9% |
| duplicate/cache_8/miss_replace | 10.74 [10.55, 11.00] | 10.65 [10.59, 10.71] | -0.8% |
| forward/routes_2/direct_last/497 | 31.68 [31.43, 31.92] | 31.59 [31.45, 31.74] | -0.3% |
| forward/routes_2/direct_last/64 | 22.69 [22.40, 23.00] | 22.94 [22.73, 23.16] | +1.1% |
| forward/routes_2/learned_last/497 | 30.25 [30.03, 30.48] | 30.50 [30.29, 30.73] | +0.8% |
| forward/routes_2/learned_last/64 | 21.91 [21.58, 22.28] | 21.63 [21.47, 21.79] | -1.3% |
| forward/routes_32/direct_last/497 | 36.91 [36.58, 37.28] | 36.89 [36.66, 37.12] | -0.0% |
| forward/routes_32/direct_last/64 | 29.76 [29.44, 30.10] | 29.78 [29.48, 30.07] | +0.1% |
| forward/routes_32/learned_last/497 | 39.76 [39.44, 40.10] | 40.70 [40.40, 41.01] | +2.4% |
| forward/routes_32/learned_last/64 | 33.54 [32.84, 34.36] | 32.48 [32.05, 32.95] | -3.2% |
| forward/routes_8/direct_last/497 | 34.67 [34.44, 34.92] | 34.64 [34.49, 34.77] | -0.1% |
| forward/routes_8/direct_last/64 | 25.39 [25.19, 25.60] | 25.22 [25.04, 25.40] | -0.7% |
| forward/routes_8/learned_last/497 | 33.07 [32.89, 33.26] | 32.88 [32.72, 33.02] | -0.6% |
| forward/routes_8/learned_last/64 | 23.04 [22.87, 23.20] | 23.58 [23.35, 23.82] | +2.3% |
| saturated/npdu_1497/queue_1 | 30.50 [30.32, 30.68] | 30.51 [30.40, 30.62] | +0.0% |
| saturated/npdu_1497/queue_4 | 30.86 [30.58, 31.27] | 30.54 [30.37, 30.71] | -1.0% |
| saturated/npdu_1497/queue_8 | 30.82 [30.55, 31.13] | 30.39 [30.21, 30.58] | -1.4% |
| saturated/npdu_501/queue_1 | 13.22 [13.03, 13.46] | 13.07 [13.01, 13.14] | -1.1% |
| saturated/npdu_501/queue_4 | 13.10 [13.02, 13.17] | 13.18 [13.09, 13.28] | +0.7% |
| saturated/npdu_501/queue_8 | 13.07 [13.01, 13.13] | 13.05 [12.98, 13.12] | -0.2% |
