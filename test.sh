#!/bin/bash

set -euxo pipefail

cargo test

cargo miri test --tests
cargo miri test --target i686-unknown-linux-gnu --tests

# both examples and docs contain things that miri isn't a fan of
# cargo miri test --doc