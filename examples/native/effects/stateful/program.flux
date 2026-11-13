v2
(pub smooth (sample alpha previous:f32) f32 = (+ previous (* alpha (- sample previous))))
(pub residual (sample low:f32) f32 = (- sample low))
(pub envelope (sample alpha previous:f32) f32 = (+ previous (* alpha (- (abs sample) previous))))
(pub gain (sample level:f32) f32 = (* sample level))
