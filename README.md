# `icon`

Reality-compliant rust crate to find icons on linux with ease.

## Quickstart

`cargo add icon`

Using the default configuration (should suit most usecases):

```rust
use icon::Icons;

let icons = Icons::new();
let firefox = icons.find_default_icon("firefox", 64, 1);
```

It's as easy as that.
