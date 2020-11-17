local th = sludge.thread

function spiral()
    th.spawn(function()
        local ring = danmaku.ring(8, 2)
        local group = danmaku.group()
    
        for t=1,32 do
            n = sludge.math.tanh(math.sin(t / 128 * 6 * math.pi))
    
            danmaku.spawn("EasedBullet", function(builder)
                builder:duration(5)
                builder:destination(32, 0)

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
            end, group)
            
            th.yield(5)
        end

        th.yield(180)

        local pattern = group:to_pattern():of(danmaku.aimed(50, 50))
        danmaku.spawn("TestBullet", function(builder)
            builder:add_linear_velocity(60, 0)
            pattern:build(builder)
        end)
        group:cancel()
    end)
end

th.spawn(function()
    local s = spiral

    while true do
        spiral()
        th.yield(1000)
    end
end)
