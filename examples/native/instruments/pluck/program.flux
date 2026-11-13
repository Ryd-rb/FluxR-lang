v2
(fn wave (phase:f32) f32 = (? (< phase 0.35f) 1f -0.5384615f))
(fn envelope (age:f32) f32 = (? (< age 0.04f) (/ age 0.04f) (* (- 1f age) (- 1f age))))
(pub sample (phase age velocity:f32) f32 = (* (* (wave phase) velocity) (envelope age)))
