local Bullet = sludge.templates.Bullet

local thread = sludge.thread.create(function()
    while true do
        for i=1,5 do
            -- Can be made with from_table or constructor
            Bullet(
                160, 120,
                60 * (math.random() - 0.5), 60 * (math.random() - 0.5)
            )

            Bullet:from_table {
                Spatial = {
                    pos = {160, 120},
                    vel = {60 * (math.random() - 0.5), 60 * (math.random() - 0.5)},
                    acc = {0, 0},
                },
            }
        end

        sludge.thread.yield(1)
    end
end)

sludge.thread.spawn(thread)