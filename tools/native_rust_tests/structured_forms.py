# Independently authored canonical FluxR native implementations paired with structured_semantics.rs.
FORMS={
'arithmetic':'''native-fn arithmetic 2 a i32 b i32 i32 1
tail bin rem bin sub bin mul bin add path a path b int 3 bin div path a path b int 7
''',
'boolean':'''native-fn boolean 2 a i32 b i32 bool 1
tail bin and bin and bin and bin ne path a path b bin or bin lt path a path b bin ge path a path b not bin eq path a path b bin or bin le path a path b bin gt path a path b
''',
'bits':'''native-fn bits 2 a u32 b u32 u32 1
tail bin xor bin bitor bin shl path a int 2 bin shr path b int 1 bin bitand path a path b
''',
 'tuple_array_index':'''native-fn tuple_array_index 0 "(i32, usize)" 2
let values array 3 int 2 int 7 int 11
tail tuple 2 index path values int 1 method path values len 0
''',
 'field_and_references':'''native-fn field_and_references 0 i32 4
var record Record rust 20
Record { number: 3 }
let number mutref field path record number
set deref path number bin add deref path number int 4
tail deref ref field path record number
''',
 'native_closures':'''native-fn native_closures 0 i32 2
let offset int 7
tail bin add call apply 2 int 5 move 1 x bin add path x path offset call apply 2 int 3 closure 1 x bin mul path x int 2
''',
 'closure_cast':'''native-fn closure_cast 0 Callback 1
tail cast move 0 int 42usize Callback
''',
 'flow':'''native-fn flow 0 i32 4
var total i32 int 0
for i bin range-inclusive int 0 int 5 3
if-stmt bin eq path i int 2 1 continue 0
if-stmt bin eq path i int 5 1 break 0
set path total bin add path total path i
while bin lt path total int 10 1 set path total bin add path total int 1
return path total
''',
 'block_if_negation':'''native-fn block_if_negation 1 flag bool i32 2
let-type result i32 block 1 let temp neg int 4 if path flag path temp neg path temp
tail path result
''',
 'result_propagation':'''native-fn result_propagation 1 value "&str" "Result<i32, std::num::ParseIntError>" 2
let-type number i32 try method path value parse 0
tail call Ok 1 bin add path number int 1
''',
 'async_await':'''native-asyncfn async_await 0 usize 2
let value async int 41usize
tail bin add await path value int 1
''',
 'async_move':'''native-asyncfn async_move 0 usize 2
let value int 7usize
tail await async-move bin add path value int 1
''',
 'unsafe_pointer':'''native-fn unsafe_pointer 0 i32 3
let value int 19i32
let pointer cast ref path value "*const i32"
tail unsafe deref path pointer
''',
 'call_evaluation_order':'''native-fn call_evaluation_order 1 log "&mut Vec<i32>" i32 1
tail call std::cmp::max 2 call touch 2 path log int 1 call touch 2 path log int 2
''',
 'assignment_order':'''native-fn assignment_order 1 log "&mut Vec<i32>" unit 1
set deref call target 1 path log call touch 2 path log int 1
''',
 'short_circuit':'''native-fn short_circuit 1 counter "&AtomicUsize" bool 1
tail bin or bin and bool false bin eq method path counter fetch_add 2 int 1 path Ordering::SeqCst int 0 bool true
''',
 'strings_and_macro':'''native-fn strings_and_macro 0 String 1
tail macro format 3 str "{}:{}" str "héllo\\n\\t\\u0000\\\"\\\\🌍" int 42
''',
 'public_value':'''native-pubfn public_value 0 i32 1 tail int 23
''',
 'public_async':'''native-pubasyncfn public_async 0 i32 1 tail int 24
''',
 'units_and_one_tuple':'''native-fn units_and_one_tuple 0 "((),)" 1 tail tuple 1 unit
'''
}
