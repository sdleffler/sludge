local th = sludge.thread

function spiral()
    th.spawn(function()
        local ring = danmaku.ring(8, 2)
    
        for t=1,32 do
            n = sludge.math.tanh(math.sin(t / 128 * 6 * math.pi))
    
            danmaku.spawn("TestBullet", function(builder)
                builder:add_linear_velocity(30, 0)
                builder:add_linear_accel(-5, 0)

                builder:translate(320 / 2, 240 / 2)
                builder:rotate(t / (81 * 3) * math.pi * 2 * 6)
                local theta = n * math.pi * 2 / 3;
    
                builder:push()
                    builder:rotate(theta)
                    ring:build(builder)
                builder:pop()
    
                builder:push()
                    builder:rotate(-theta)
                    ring:build(builder)
                builder:pop()
            end)
            
            th.yield(5)
        end
    end)
end

th.spawn(function()
    local s = spiral

    while true do
        spiral()
        th.yield(1000)
    end
end)
