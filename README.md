# Token Sale Program

An Anchor/Solana program for running a **fixed-price token sale** within a defined
time window. Participants pay USDC into a program-controlled vault and receive the
sale token at a price the operator sets in cents. The operator funds the inventory,
opens and closes the window, can adjust the price, and withdraws proceeds.

This is a *primary sale* — the token has no exchange price yet; the operator declares
one. The design favors a simple, auditable trust model over decentralization (see
[Trust model](#trust-model)).

## How it works

A single PDA derived from the seed `"sale_config"` holds the sale's configuration and
acts as the signing authority over two token vaults:

```
                 sale_config (PDA, seed = "sale_config")
                  ├─ config: admin, mints, price, decimals, window
                  └─ authority over ↓
              vault_usdc                 vault
            (USDC received)        (tokens for sale)
```

Both vaults are ordinary SPL token accounts whose `owner` is the PDA. Since a PDA has
no private key, only the program can move funds out of them — it signs CPIs with the
seed + bump. Buyers therefore deposit into the vault without the operator being able to
intercept the atomic swap.

## Instructions

| Instruction    | Caller        | Effect |
|----------------|---------------|--------|
| `initialize`   | operator (once) | Creates the `sale_config` PDA: mints, price-in-cents, decimals, and the `[start_ts, end_ts)` window. |
| `purchase`     | any buyer     | **Only while the window is open.** Pulls the USDC cost from the buyer into `vault_usdc`, then program-signs `amount` tokens out of `vault` to the buyer. Both legs are atomic. |
| `update_price` | operator      | Sets a new price (takes effect immediately). |
| `withdraw`     | operator      | Moves USDC and/or tokens from the vaults back to the operator — **not** time-gated, so unsold inventory is always recoverable. |
| `close`        | operator      | Closes both vaults (rent back to operator) and the config account. Vaults must be empty, so withdraw first. |

### Sale window

`purchase` requires `start_ts ≤ now < end_ts` (checked against the on-chain `Clock`),
otherwise it rejects with `SaleNotStarted` / `SaleEnded`. The window only restricts
**buying** — `withdraw` and `close` stay open, so when the sale ends the operator
simply withdraws whatever didn't sell. Nothing is ever locked.

## Pricing

```
cost_usdc = (amount × price_cents × 10^usdc_decimals) / (10^token_decimals × 100)
```

Example — 100 tokens at $0.01, token decimals 9, USDC decimals 6:

```
numerator   = 100e9 × 1 × 1e6 = 1e17
denominator = 1e9 × 100       = 1e11
cost        = 1e17 / 1e11     = 1_000_000 = 1.00 USDC
```

All multiplications are done in `u128` (the numerator can exceed `u64`), divided, then
narrowed back to `u64` with a checked conversion. Any overflow returns `MathOverflow`
instead of wrapping. The cost is floored; a purchase that rounds to 0 is rejected
(`BelowMinimumPurchase`).

## Events

The program emits events for off-chain indexers/frontends: `SaleInitialized`,
`SalePurchase`, `PriceUpdated`, and `SaleClosed`.

## Security notes

- **Vault ownership** — `purchase` and `close` assert `vault.owner == sale_config` so a
  caller can't substitute an account they control and drain it via the PDA's signature.
- **Mint binding** — buyer token accounts are constrained to the mints recorded at
  `initialize`; a mismatched mint is rejected.
- **Authority** — `update_price`, `withdraw`, `close` check the signer against the stored
  admin. The admin is fixed at init (no transfer instruction).
- **Checked arithmetic** — pricing uses `checked_*` in `u128` throughout.
- **Atomicity** — USDC-in and tokens-out happen in one instruction; either both apply or
  the transaction reverts.

## Trust model

Deliberate trade-offs, stated plainly rather than hidden:

- **Operator-trusted.** One admin key controls price, withdrawals, and closing. Suitable
  for an issuer-run sale; it is a centralization point.
- **Price can move mid-sale.** `update_price` is immediate, so a buy settles at the price
  current when it lands. To let buyers cap their cost, add a `max_usdc_cost` argument and
  assert against it.
- **Inventory = what's funded.** No on-chain cap; the operator funds the token vault, and
  sales stop when it's empty. Ending and withdrawing are manual.
- **Classic SPL Token** only (not Token-2022).

## Toolchain

These versions are a known-good, mutually-compatible set. Solana and Anchor are tightly
coupled, so matching versions matters more than using the newest of each.

| Tool | Version | Notes |
|------|---------|-------|
| Anchor CLI | 0.31.1 | via `avm` |
| Solana / Agave CLI | 2.1.0 | platform-tools v1.43 → on-chain (SBF) compiler rustc 1.79 |
| Host Rust | 1.86 | IDL generation + tests |
| Node.js | 20 LTS | pinned in `.nvmrc` |

The program and the IDL/tests are produced by **two different compilers** — the SBF
compiler (from the Solana CLI) for the deployable `.so`, and host `rustc` for IDL
generation and the TypeScript tests — so both must be satisfied. Crate versions are
locked in `Cargo.lock`.

## Build & test

```bash
nvm use          # Node 20, from .nvmrc
yarn install
anchor build     # compile program + generate IDL
anchor test      # spins up a local validator, deploys, runs tests/, tears down
```

## Status

Builds cleanly and passes the full test suite on a local validator. **Not deployed to a
public cluster.** When deployed, record it here:

```
Cluster / program ID: <PROGRAM_ID>
Explorer: https://explorer.solana.com/address/<PROGRAM_ID>?cluster=devnet
```
