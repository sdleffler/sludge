# SLUDGE - Lua-based extensions to the HECS ECS for scripting and serialization

## Lua API

### `sludge`

#### Functions

```lua
-- Create a new template with the provided string name.
sludge.Template(name)
```

### `sludge.log`

#### Functions

Logging which delegates to the Rust `log` crate under the hood.

```lua
-- Log a message with the indicated log level. If the function has two parameters,
-- then the first is assumed to be a string target.
sludge.log.trace(target, message), sludge.log.trace(message)
sludge.log.debug(target, message), sludge.log.debug(message)
sludge.log.info(target, message), sludge.log.info(message)
sludge.log.warn(target, message), sludge.log.warn(message)
sludge.log.error(target, message), sludge.log.error(message)

-- Log a message with the given parameter table. `parameter` must be either a
-- string log level or a table `{ level = ..., target = ... }`.
sludge.log.log(parameters, message)
```

### `sludge.math`

#### Functions

```lua
-- Create a new `Transform` object, initialized to the identity transform.
sludge.math.Transform()
```