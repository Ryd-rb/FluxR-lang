v2
(pub sample (phase age velocity:f32) f32 (state:i64 noise:f32)
 (= state (% (+ (* (ftoi64 (* phase 65535f)) 1103515245) 12345) 65536))
 (= noise (- (/ (i64tof state) 32768f) 1f))
 (ret (* velocity (* noise (* (- 1f age) (* (- 1f age) (- 1f age)))))))
