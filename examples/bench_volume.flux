v2
(fn lerp (a:i64 b:i64 t:i64) i64 ()
  (ret (+ a (/ (* (- b a) t) 1024))))
(pub volume (a:i64 b:i64 c:i64 d:i64 e:i64 f:i64 g:i64 h:i64 x:i64 y:i64 z:i64) i64
  (near:i64 far:i64)
  (= near (call lerp (call lerp a b x) (call lerp c d x) y))
  (= far (call lerp (call lerp e f x) (call lerp g h x) y))
  (ret (call lerp near far z)))
