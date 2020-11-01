return function(accessor)
    local status, tt = pcall(function() return accessor.to_table end)
    if status and tt then
        return tt(accessor)
    else
        return true
    end
end