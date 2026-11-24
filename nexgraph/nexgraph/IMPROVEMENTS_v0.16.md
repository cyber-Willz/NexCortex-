# nexgraph swarm.detect: test on a public dataset (addendum to v0.15)
Scope: detection/characterisation of coordinated tracks only; nothing for engagement or targeting.

## Data
UAM-FD "UAMTra2Flow" (public GitHub): 48 real Crazyflie runs (2/4/6/8 drones, 50 s, 10 Hz, ~2 m arena). Drones fly to independent destinations, so it contains **no coordinated swarm**: it is a real false-alarm control and a real background for planted formations. Planted swarms (5 drones = a real flown trajectory + fixed offsets sd 0.25 m + Gaussian noise) are injected, with ground truth. 12 backgrounds per cell, eps=1.5 m.

## Results (unchanged detector)
* False alarms: 0 groups in 36 files (D4/D6/D8) at windows 2, 5, 10 s, default thresholds, and also with loosened cos/persist.
* Easy cases (sd 0.01-0.12 m; rigid, lagged followers, 40% dropout, breathing): 12/12 exact at 5 s window (2 breathing misses).
* Harder cases, window 5 s, noise sd 0.05 / 0.3 / 0.6 m: single swarm 12 / 10 / 10 of 12 exact; 3-drone 12 / 10 / 10; rotating 11 / 10 / 9; two swarms 24 / 18 / 17 of 24; escort drone 11 / 9 / 10. 0 false groups throughout.
* Window 2 s collapses (noise 0.3: 3-5 of 12; 0.6: 0 of 12). Window length is the dominant factor.
* Every miss I inspected was a slow or hovering leader (median window speed 0.04-0.08 m/s vs 0.4 m/s typical): velocity has no power when motion is below noise. Not a threshold problem.
* Two swarms flying side by side 0.9 m apart merge into one group (12/12), which is the documented connected-components limit; separation is within the formation's own spread so truth is arguably ambiguous.

## Tried and rejected (do not ship)
* Noise-adaptive displacement-consistency link (chi-square on pair displacement): no better than baseline at any window.
* Rigid-separation link (no velocity needed, works for hovering): **14 false groups on the 36 real uncoordinated files**, because parked drones are rigid. Unusable as a swarm verdict.

## Shipped
`swarm.detect` summary row now reports `low_motion_track_windows=N of M`: track-windows whose speed is under 3 sigma of the least-squares velocity error (sigma from the window's own residuals). No other output changed; all swarm tests pass (6, including a swarm.scan test another session added). On a planted sigma 0.6 case: win 2 -> 140/225 low-motion, 0 groups; win 5 -> 26/90, found; win 10 -> 1/45, found. Rule of thumb: if low-motion exceeds ~30%, a "no group" answer is not evidence of absence; lengthen the window or use swarm.scan.

## Limits
Planted swarms are synthetic on real backgrounds; no real coordinated-swarm ground truth was available. One arena, one speed regime. Slow/hovering coordinated formations remain undetectable by this method without accepting false alarms. Code largely AI-generated; review before operational use.
