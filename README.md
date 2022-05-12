# clone-replace - share data by copying a reference

[CloneReplace] provides an evolving reference version of some
data. When the data is accessed, you are returned an [Arc] handle to a
snapshot of the reference as it existed at that moment. When you wish
to mutate your data, a full copy is made of it, which you can update
independently, without blocking any readers. Upon completing your
modifications, the copy will be written back to become the new
reference version.

Example:

```rust
use clone_replace::CloneReplace;

let data = CloneReplace::new(1);

let v1 = data.access();
assert_eq!(*v1, 1);
{
   let mut m = data.mutate();
   *m = 2;
   let v2 = data.access();
   assert_eq!(*v1, 1);
   assert_eq!(*v2, 1);
}
let v3 = data.access();
assert_eq!(*v3, 2);
assert_eq!(*v1, 1);
```

## License

This crate is made available under either an
[Apache-2.0](https://opensource.org/licenses/Apache-2.0) or an [MIT
license](https://opensource.org/licenses/MIT).
