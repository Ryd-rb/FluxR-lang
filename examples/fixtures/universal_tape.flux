v2
(rec Rule next:u64 write:u64 right:bool)
(rec Tape state:u64 head:u64 left:[u64] right:[u64])
(inputs alphabet:u64 rules:[Rule])
(pub advance (t:Tape steps:u64) Tape (state:u64 head:u64 left:[u64] right:[u64] r:Rule)
  (= state t.state)
  (= head t.head)
  (= left t.left)
  (= right t.right)
  (while (&& (< 0u steps) (< 0u state)) (
    (= r (at (input rules) (+ (* (- state 1u) (input alphabet)) head)))
    (= state r.next)
    (= head 0u)
    (if r.right (
      (push left r.write)
      (if (! (empty right)) ((= head (last right)) (pop right))))
    (
      (push right r.write)
      (if (! (empty left)) ((= head (last left)) (pop left)))))
    (= steps (- steps 1u))))
  (ret (new Tape state head left right)))
