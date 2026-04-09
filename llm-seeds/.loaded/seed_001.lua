Here are the 15 seed scripts:

-- 1: __gc resurrects dead object into global table
local graveyard = {}
local function make()
  local t = newproxy(true)
  local mt = getmetatable(t)
  mt.__gc = function(self)
    graveyard[#graveyard + 1] = self
    collectgarbage("collect")
    local _ = tostring(graveyard[#graveyard])
  end
  return t
end
for i = 1, 20 do make() end
collectgarbage("collect")
collectgarbage("collect")
for i = 1, #graveyard do
  local _ = tostring(graveyard[i])
end
collectgarbage("collect")
local x = graveyard[1]
graveyard = nil
collectgarbage("collect")
local _ = tostring(x)
