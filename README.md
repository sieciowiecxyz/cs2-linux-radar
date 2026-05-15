# cs2-radar

Standalone host-side radar stack for CS2.

## Layout
- `src/apps/fun` - launcher
- `src/fun/radar` - web radar
- `src/fun/trigger` - trigger decision lane
- `src/fun/mouse` - mouse relay
- `src/game/cs2` - CS2 runtime reader
- `src/game/deadlocked/headless` - headless helpers used by `cs2`
- `src/host/kmod-memreader` - kernel module, uapi, and client
- `assets/radar` - web UI and map assets

## Run
```bash
cargo run -p fun -- cs2
```

## Notes
- `fun-mouse` is trigger-only here.
- `fun-radar` loads `/dev/memreader` via `src/host/kmod-memreader/module`.
- `scripts/radar` starts the stack in the background and logs to `/tmp/fun.log`.
