# J. Schema Stack Empirical Verification — for Quillon

*Research date: 2026-11-17. Verified by compiling and running real Rust code in this sandbox (schemars 1.2.3, serde 1.0.229, serde_json 1.0.154, jsonschema 0.61.2). This is measured fact, not documentation claim.*

## What was tested

A tagged enum `Element { Text{..}, Rect{..}, Image{..} }` with `#[serde(tag = "type", rename_all = "snake_case")]` + `#[derive(JsonSchema)]`, the exact element-model pattern Quillon's document format will use.

## Result 1 — schemars generates a clean discriminated-union schema

`schemars::schema_for!(Element)` emits draft **2020-12** with `oneOf` per variant, and each variant carries `"type": {"const": "text"}` (etc.) as the discriminator — exactly the shape agents and constrained-decoding runtimes want. Verified output excerpt:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "oneOf": [
    { "type": "object",
      "properties": {
        "type": {"type": "string", "const": "text"},
        "text": {"type": "string"},
        "font": {"type": "string"} },
      "required": ["type", "text", "font"] },
    ...
  ]
}
```

## Result 2 — serde's own error messages are already actionable

`serde_json::from_str::<Element>(r#"{"type":"rect","x":1,"y":2,"w":10}"#)` → **`missing field \`h\`** — field-qualified, no pointer to internals. Good for the Rust API path.

## Result 3 — jsonschema oneOf errors are NOT yet agent-grade (design implication)

Feeding the schemars-generated schema to `jsonschema::validator_for` and validating the same bad document yields:

```
"{...} is not valid under any of the schemas listed in the 'oneOf' keyword at "
```

No JSON path, no "which variant matched closest, which field is missing". This is the known oneOf weakness (JSON Schema discriminator proposals; ajv's discriminator support).

**Design decision for Quillon**: never return raw jsonschema oneOf errors to agents. Validation pipeline:
1. Deserialize with serde first → field-qualified errors (`missing field h`) — free and better.
2. If serde passes but schema-level constraints matter (e.g., cross-field), run jsonschema and map its errors.
3. Wrap both into our own `ValidationError { path, code, message }` shape where path is the JSON pointer (e.g. `/scenes/0/elements/3`) — this is the actionable-error property Clipkit credits for its 100% vs 87% agent-validity gap (research/C §2).
4. Known schemars/serde quirks to respect: `#[serde(with)]` needs real types + explicit generics; `#[serde(bound)]` is ignored by schemars (restate via `#[schemars(bound)]`); internally-tagged enums put the tag first in `required` but property order differs (schemars issue #537, cosmetic).

## Bonus verification — toolchain works end-to-end in this sandbox

cargo 1.100.0 compiled schemars + serde + jsonschema and ran the test binary. Build times acceptable (~10s cold). The Quillon workspace will build on the installed toolchain without extra setup.
