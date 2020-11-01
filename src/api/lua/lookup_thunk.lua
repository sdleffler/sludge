-- Defer looking up an entity in a persisted entity ID table to preserve uniqueness.
return function(entity_id, world_table)
    return function()
        return world_table[entity_id]
    end
end