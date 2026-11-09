v2
(pub decide (a:i64 b:i64 c:i64 x:i64) i64 ()
  (if (< a 0)
    ((if (< b 0)
      ((if (< c 0) ((ret (+ x 1))) ((ret (- x 1)))))
      ((if (< c 0) ((ret (* x 2))) ((ret (/ x 2)))))))
    ((if (< b 0)
      ((if (< c 0) ((ret (+ x 3))) ((ret (- x 3)))))
      ((if (< c 0) ((ret (* x 4))) ((ret (/ x 4)))))))))
