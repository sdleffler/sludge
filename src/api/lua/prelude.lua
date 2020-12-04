local yield = sludge.thread.yield
local status = sludge.thread.status

function sludge.thread.wait_until(predicate)
    while not predicate() do
        yield(1)
    end
end

function sludge.thread.join(...)
    repeat
        for i = 1, select("#", ...) do
            if status(select(i, ...)) ~= "dead" then
                goto continue
            end
        end

        do return end
        
        ::continue::
        yield(1)
    until false
end