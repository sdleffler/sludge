local th = sludge.thread

local M = {}

local exp = math.exp

function M.cosh (x)
  if x == 0.0 then return 1.0 end
  if x < 0.0 then x = -x end
  x = exp(x)
  x = x / 2.0 + 0.5 / x
  return x
end


function M.sinh (x)
  if x == 0 then return 0.0 end
  local neg = false
  if x < 0 then x = -x; neg = true end
  if x < 1.0 then
    local y = x * x
    x = x + x * y *
        (((-0.78966127417357099479e0  * y +
           -0.16375798202630751372e3) * y +
           -0.11563521196851768270e5) * y +
           -0.35181283430177117881e6) /
        ((( 0.10000000000000000000e1  * y +
           -0.27773523119650701667e3) * y +
            0.36162723109421836460e5) * y +
           -0.21108770058106271242e7)
  else
    x =  exp(x)
    x = x / 2.0 - 0.5 / x
  end
  if neg then x = -x end
  return x
end


function M.tanh (x)
  if x == 0 then return 0.0 end
  local neg = false
  if x < 0 then x = -x; neg = true end
  if x < 0.54930614433405 then
    local y = x * x
    x = x + x * y *
        ((-0.96437492777225469787e0  * y +
          -0.99225929672236083313e2) * y +
          -0.16134119023996228053e4) /
        (((0.10000000000000000000e1  * y +
           0.11274474380534949335e3) * y +
           0.22337720718962312926e4) * y +
           0.48402357071988688686e4)
  else
    x = exp(x)
    x = 1.0 - 2.0 / (x * x + 1.0)
  end
  if neg then x = -x end
  return x
end

function spiral()
    th.spawn(function()
        local ring = danmaku.ring(8, 2)
    
        for t=1,32 do
            n = M.tanh(math.sin(t / 128 * 6 * math.pi))
    
            danmaku.spawn("TestBullet", function(builder)
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
