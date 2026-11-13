v2
(fn wave (phase:f32) f32 = (? (< phase 0.5f) (- (* phase 4f) 1f) (- 3f (* phase 4f))))
(pub sample (phase age velocity:f32) f32 = (* (* (wave phase) velocity) (* (- 1f age) (- 1f age))))
