
-- 2: Weak table whose values have __gc referencing the weak table
local weak = setmetatable({}, {__mode = "v"})
for i = 1, 30 do
  local t = newproxy(true)
  local mt = getmetatable(t)
  mt.__gc = function(self)
    local n = 0
    for k, v in pairs(weak) do n = n + 1 end
    weak[tostring(n)] = {n}
    collectgarbage("step", 1)
  end
  weak[i] = t
end
collectgarbage("collect")
collectgarbage("collect")
for k, v in pairs(weak) do
  local _ = tostring(k) .. tostring(v)
end
collectgarbage("collect")
