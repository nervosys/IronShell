# IronShell Standard Library

The standard library provides commonly used functions and utilities organized into logical modules.

--

## Modules

| Module       | Description                                          |
| ------------ | ---------------------------------------------------- |
| `prelude`    | Most commonly used functions (auto-imported in REPL) |
| `math`       | Mathematical functions and constants                 |
| `string`     | String manipulation functions                        |
| `collection` | Collection manipulation utilities                    |
| `functional` | Functional programming utilities                     |
| `io`         | File I/O and data serialization                      |

## Usage

```ish
# Import modules
import "lib/prelude.ish"
import "lib/math.ish"
import "lib/string.ish"
import "lib/collection.ish"
import "lib/functional.ish"
import "lib/io.ish"

# Use imported functions
PI                          # 3.14159...
is_even(4)                  # true
words("hello world")        # ["hello", "world"]
union([1,2], [2,3])         # [1,2,3]
find([1,2,3,4], fn(x) => x > 2)  # 3
```

## Module Overview

### prelude.ish
Core utilities that most programs need:
- `id` - Identity function
- `compose` - Function composition (right-to-left)*
- `pipe_fn` - Function composition (left-to-right)*
- `flip` - Flip function arguments*
- `not`, `both`, `either` - Boolean utilities
- `is_some`, `is_none`, `get_or` - Option utilities
- `clamp` - Clamp value to range

*Note: Higher-order functions that return lambdas may have closure issues when imported.

### math.ish
Mathematical utilities:
- `PI`, `E`, `PHI`, `TAU` - Mathematical constants
- `is_even`, `is_odd` - Parity checking
- `is_positive`, `is_negative`, `is_zero` - Sign checking
- `sign` - Sign of a number (-1, 0, or 1)
- `lerp`, `inv_lerp` - Linear interpolation
- `square`, `cube` - Power functions
- `gcd`, `lcm` - Number theory
- `factorial`, `fib` - Sequences (a negative n is refused)
- `deg_to_rad`, `rad_to_deg` - Angle conversion
- `mean` is the builtin (`avg`), not a lib function: `[1, 2] | mean` is `1.5`

### string.ish
String manipulation utilities:
- `words`, `unwords` - Split on any whitespace run (spaces, tabs, newlines, CR), drop empty words / join with spaces
- `unlines` - Join with newlines. `lines` is the builtin (it drops the final terminator and CR before LF); this library does not define it, because an import would shadow the builtin
- `capitalize`, `title_case` - Case transformation
- `snake_case`, `kebab_case`, `screaming_snake` - Case styles
- `is_blank`, `is_not_blank` - Blank checking
- `is_upper`, `is_lower` - Case checking: true only with at least one cased letter and no letter of the other case (`"123"` and `""` are false), as Python's `str.isupper`
- `repeat` - Repeat n times (a negative n is refused), `reverse_str`, `count_substr` (non-overlapping; an empty substring counts len + 1 positions, as Python's `str.count`)

### collection.ish
Collection manipulation:
- `partition` - Split by predicate
- `union`, `intersect`, `difference` - Set operations; elements are compared with `==`, so `1` and `1.0` are one element (the builtin `unique` keeps both)
- `is_subset` - Subset checking
- `sort_desc` - Reverse sort

### functional.ish
Advanced functional programming utilities:
- `curry`, `uncurry` - Currying functions*
- `partial`, `partial_right` - Partial application*
- `twice` - Apply function twice*
- `complement`, `and_p`, `or_p` - Predicate combinators*
- `drop` - Drop first n elements (a negative n is refused)

There is no `find` here: it would shadow the builtin `find` (file search) on import. For the first element matching a predicate, write `arr | where(pred) | first`.

*Note: Functions that return lambdas may have closure issues when imported.

### io.ish
Input/output utilities:
- `read_json` - Read JSON file
- `read_lines`, `read_trimmed` - Read text files
- `file_ext`, `file_name`, `file_dir` - Path utilities. They work on the last component: `file_ext("dir/file")` is `""`, `file_ext(".bashrc")` is `""`, `file_name("a/b/")` is `"b"`, `file_dir("/x")` is `"/"`
- `path_join` - Join path components with one slash at each joint (`path_join(["a/", "/b"])` is `"a/b"`)
- `to_json_pretty` - Pretty-printed JSON (two spaces per level); `to_json(v)` is compact
- `parse_yaml`, `parse_csv` - Data parsing

## Testing

Run the standard library tests:
```bash
./target/debug/ish lib/test_stdlib.ish
```
