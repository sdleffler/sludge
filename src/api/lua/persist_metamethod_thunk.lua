return function(thunk)
    return setmetatable({}, {
        __persist = function(_) return thunk end,
    })
end