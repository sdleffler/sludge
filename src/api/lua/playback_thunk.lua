return function(orig_world_table)
    -- Shallow copy of the world table to avoid recursion w/ Eris.
    local world_table = {}
    for i,v in ipairs(orig_world_table) do
        world_table[i] = v
    end

    return function()
        local spawn = sludge.spawn
        for _,v in ipairs(world_table) do
            world_table[v.id] = (v.deserialize or spawn)(v.components)
        end

        return world_table
    end
end