-- This is a small thunk which works as a userdata persister, by converting a
-- Lua value in Rust into a small closure which simply returns the same value.
-- This allows us to easily generate a closure which persists an arbitrary
-- Lua value from Rust.
--
-- In essentia it just creates a closure which wraps and returns a closure that
-- returns some object. It's a hack, okay?
return function(object)
    return function() return object end
end