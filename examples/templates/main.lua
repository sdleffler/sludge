-- Load the "Bullet" game object template.
local Bullet = sludge.templates.Bullet

-- Create a new task that will run alongside the main game update functions.
local thread = sludge.thread.create(function()
    -- Repeat this forever.
    while true do
        -- Repeat this 50 times, spawning 2 bullets each time, to spawn 100 bullets.
        for i=1,50 do
            -- Can be made with from_table or constructor
            Bullet(
                 -- X, Y position
                160, 120,
                -- X, Y velocity
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

        -- Wait 60 frames.
        sludge.thread.yield(60)
    end
end)

-- Start the task.
sludge.thread.spawn(thread)