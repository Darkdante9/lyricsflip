# Generated TypeScript Bindings

This directory contains TypeScript bindings auto-generated from the
`lyricsflip` contract WASM using `stellar contract bindings typescript`.

**Do not edit these files by hand.** To regenerate:

```bash
cd onchain
./scripts/generate-bindings.sh
```

Or via the npm script:

```bash
cd frontend
npm run generate:bindings
```

## Freshness check

CI runs `onchain/scripts/check-bindings-fresh.sh` (the `bindings-check` job
in `.github/workflows/onchain.yml`) on every PR. The job rebuilds the WASM
and re-generates the bindings into a temporary directory, then diffs the
result against this directory. The check **fails** if any file differs, which
means a contract type was changed without regenerating the bindings.

If you see a CI failure here, run `npm run generate:bindings` from the
`frontend/` directory and commit the updated files.
