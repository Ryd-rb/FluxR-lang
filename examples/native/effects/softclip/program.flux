v2
(pub softclip (sample drive:f32) f32 (scaled:f32)
 (= scaled (* sample drive))
 (ret (/ scaled (+ 1f (abs scaled)))))
(pub mix (left right balance:f32) f32 = (+ (* left (- 1f balance)) (* right balance)))
