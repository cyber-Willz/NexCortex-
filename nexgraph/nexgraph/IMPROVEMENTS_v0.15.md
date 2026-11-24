# nexgraph v0.15 (platform 0.15): drone-swarm detection analytics, tested on your three archives

Scope: **detection and characterisation** of coordinated groups in track data, and analytics on drone-link telemetry. I did not build anything for engagement, targeting, countermeasures or friend / foe identification, and `swarm.detect` says nothing about intent (a legitimate formation, such as a light show, is `coordinated` too).

## What the archives contain

| file | what it is | what I did |
|---|---|---|
| `synthetic_drone_swarm_dataset.csv` (in `archive`) | 20 drones x 1,000 time steps; x, y, z, velocity, battery, signal, `collision_warning`, `formation_pattern`, `mission_status` | track analysis with the new `swarm.detect` (below) |
| `drone_communication_dataset.csv` (`archive_1`) | 45,289 hourly link records with 8 attack labels (spoofing, MITM, DDoS, GPS spoofing, malware, jamming, protocol exploit, normal) | `uw.audit` / `uw.scorecard` per label |
| `archive_2` | 300 photoreal synthetic drone images (640x640, YOLO labels, 31 empty negatives) plus a copy of the insurance-fraud graph from v0.13 | label QA only (counts, box sizes). **No image detection was run: the engine does not process pixels** |
| `archive` (ledger etc.), `archive_1` (Forbes, land registry, insurance), the fraud graph in `archive_2` | repeats of datasets covered in v0.12 to v0.14 | not re-analysed |

## What the data says

* **The swarm file is one drifting cloud, not 20 independent drones and not a formation.** The centroid random-walks from about (0,0,0) to (100,100,100) in steps of about 2.6, and each drone sits at the centroid plus a fresh, independent offset (sd 4.9 per axis, lag-1 autocorrelation 0.001) at *every* step. So drones are always within about 8 of the centroid (mean pair distance 10.8 against about 66 for random points in the box) and move together on average, but individual headings from step to step are pure jitter (mean pairwise heading cosine 0.002, order parameter 0.21 against 0.20 for random headings).
* **Its labels carry no information about the geometry.** `formation_pattern` takes all four values for every drone (and in every time step); `mission_status` likewise; `collision_warning` fires on 191 rows (1 %) whose nearest-neighbour distance (4.44) is the same as for the other rows (4.38); `velocity` is uncorrelated with the actual step length (-0.006); `signal_strength` is uncorrelated with distance from the centroid (0.006); battery is a straight decay with time (-0.995). So the dataset can test *whether a detector finds a cohesive moving group*, but it cannot validate formation classification, collision warning or mission inference.
* **Drone-link attack labels are not learnable from the features.** For all eight labels the best feature has IV <= 0.012 and AUC 0.48 to 0.52 (the best is protocol exploit on `temporal_patterns`, AUC 0.521, IV 0.012). A 22-feature scorecard for jamming has test AUC **0.501** (95 % CI 0.482 to 0.520). Jamming is not related to signal-to-noise ratio, GPS spoofing not to `gps_signal_integrity` (constant 1.0 for all rows, as is `drone_signal_handoff`). 62 % of rows carry no label at all and 7 % carry more than one. `drone_gps_coordinates` is a random point on the globe per row. No attack detector can be built or validated on it.
* **Image set (label QA):** 243 / 36 / 21 images with 267 / 38 / 31 boxes (matches its README); median box 1.1 % of image area, and 50 of the 267 training boxes are under 0.1 % of the image (about 20x20 px or smaller); the test split has no negative images. Small distant drones dominate the difficulty. The README reports mAP50 0.41 for synthetic-only training on real photos; I could not check that.

## What I built

### `swarm.detect(label, idKey, timeKey, xKey, yKey, zKey, win[, opts])`

One edge per observation. Per track and time window (`floor(t / win)`): mean position and **least-squares velocity** (this averages out position jitter; a two-point difference does not). Two tracks are linked in a window when both have a velocity, are within `eps`, their velocities have cosine >= `cos_min` and the slower speed is at least `speed_tol` of the faster. A pair is persistent when it was co-present in >= `min_windows` windows and linked in a fraction >= `persist` of them. Groups are the connected components of persistent pairs (>= `min_size`).

Per group it reports size, members, windows, persistence, mean pair distance (between window-mean positions), heading coherence (1 = parallel; about 1/sqrt(n) for unrelated headings), speed mean and CV, and two Monte Carlo p-values (deterministic splitmix64 stream):

* `p_coherence`: against independent random headings (sphere, or circle with `dim=2`);
* `p_background`: against random same-size groups drawn from the tracks actually present.

Verdicts: `coordinated` (aligned beyond chance and distinct from the background), `coordinated (whole scene)` (aligned, but the group *is* the airspace, so it cannot be told from the background), `co-located` (close, no shared motion: crowds, milling birds, busy traffic), `coherent (untested)` with `perm=0`. A proximity-only detector reports every `co-located` group as a swarm.

Why two nulls: my first version had only the background test. On your dataset it returned p = 1.0 and called a perfectly cohesive 20-drone cloud `co-located`, because every random subset of a scene in which everything moves together is also aligned. That was a false negative; the isotropic-heading test and the `(whole scene)` verdict fix it.

### Importer and platform
* **One-entity observation tables could not be imported** (the importer required two member columns, so a plain `id, t, x, y, z` table failed). `HyperOpts.allow_single_member` and platform `allow_single=true` allow it; the default stays strict so a typo in `members` is still an error.
* Platform: `swarm.detect` in the Analytics catalog (group "Airspace"), argument checks, `allow_single` parameter.
* The allow-list for tenant-callable procedures gained `swarm.`.

## Results of `swarm.detect`

| test | result |
|---|---|
| Simulation (35 tracks x 120 steps): three planted formations (8, 5, 4 drones, common velocity, jitter sd 1), 12 independent drones (one crossing a formation's path), a milling crowd of 6 | exactly three groups, each `coordinated` (p = 0.001 against both nulls, the floor for 1,000 draws), members equal to the planted sets; the crossing drone and the crowd are not reported. Milling crowd alone at a loose threshold (`eps=40, cos_min=0`): `co-located`, coherence 0.36, p_coherence 0.64 |
| Planted formations vs jitter (sd 0.5, 1, 2, 4, 8) | all three found with the right members up to sd 4; at sd 8 swarm B loses one of its 5 drones (4 of 5). A more aggressive jitter or a shorter window would lose more |
| 1,500 independent random-velocity tracks, 60 steps (90,000 observations) | **0 groups**, 0.3 s (false-alarm control; the importer takes 0.3 s) |
| Your dataset, window 10 steps | **no group** (the per-drone jitter swamps the common drift) |
| Your dataset, window 50 and 100 steps | one group of 20, coherence 0.92 and 0.99, p_coherence 0.001, p_background 1.0: `coordinated (whole scene)` |

So detection depends on the window: the right window length is set by the sensor's position noise against the group's drift speed. There is no automatic window choice; run several.

## Evidence
* `tests/swarm.rs`, 5 tests. The engine is replayed against `tests/fixtures/swarm_expected.json`, generated by `swarm_expected.py` (numpy only, same splitmix64 stream, same Fisher-Yates draw) for six configurations of the simulation (default, tight `eps`, `perm=0`, strict cosine, planar `dim=2`, crowd only) and for your dataset at three windows: group membership, windows, persistence, pair distance, coherence, speed, **both p-values** and verdicts all match (floats to 1e-9; p-values exactly). Also: the planted sets are recovered with no mixed group, a hand-worked formation (three tracks at 3 m spacing flying together, a fourth close but opposite: group a,b,c with persistence 1, coherence 1, speed 2, pair distance 4), and nine argument-error cases.
* Platform: unit tests for argument compilation and injection attempts, and the API test imports your swarm file with `allow_single=true` and runs `swarm.detect` from its catalog defaults (one group of 20, whole scene), plus the 400 when the flag is missing.
* Full suite results are at the end of this file.

## Limits (read these)
* **It needs tracks.** There is no detection from radar, RF or camera, no tracking and no data association; the engine does not process pixels, so the image set was not used for detection. Track quality (gaps, id swaps, false tracks) will decide real-world performance and was not modelled.
* **Coordinated is not hostile.** A flock of birds, a light show, a convoy and an attack swarm can all be `coordinated`. Nothing here tells them apart; only speed, altitude, size or signature features you add (outside this module) could.
* Cartesian coordinates in one length unit. Latitude / longitude / altitude are not converted. The communication file's GPS column is random points, so it could not be used even if they were.
* Cost is O(tracks^2) per window (1,500 tracks: 0.3 s); a hard cap `max_tracks` (default 2,000 per window) refuses larger windows rather than running for hours. No spatial index.
* Pairs are judged window by window with one threshold set; there is no handling of merging, splitting or leaders and followers, and groups are connected components, so one persistent link can chain two formations together.
* Validated on one clean simulation (constant velocities, Gaussian jitter), a synthetic cloud, and random independent tracks. Not validated on real tracks, birds, dense air traffic or evasive behaviour. Treat thresholds as starting points.
* Monte Carlo p-values have a floor of 1/(perm + 1) (0.001 at the default); `p_max` below that can never be met.
* The telemetry findings describe a synthetic set with random labels; they say nothing about real attack detection.
* `frontend/dist` was not rebuilt. Compiled and tested with rustc 1.97 only. Code is largely AI-generated; review before any operational use.

## Suite result

Engine (`cargo test --release --no-fail-fast --features server -- --test-threads=1`, PostgreSQL 16 for the PG tests) and platform (`cargo test --release -- --test-threads=1`, `NGP_TEST_PG_URL` set): every test passes, including the new `tests/swarm.rs` (5) and the extended platform `underwriting_through_the_api`. Run the PG-backed tests single-threaded, as noted in v0.13.
