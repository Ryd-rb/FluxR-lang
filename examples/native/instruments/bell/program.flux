v2
(fn triangle (phase:f32) f32 = (? (< phase 0.5f) (- (* phase 4f) 1f) (- 3f (* phase 4f))))
(fn wrap (phase:f32) f32 = (- phase (i64tof (ftoi64 phase))))
(pub sample (phase age velocity:f32) f32 (fundamental partial envelope:f32)
 (= fundamental (triangle phase))
 (= partial (triangle (wrap (* phase 3f))))
 (= envelope (* (- 1f age) (* (- 1f age) (- 1f age))))
 (ret (* velocity (* envelope (+ (* fundamental 0.75f) (* partial 0.25f))))))
