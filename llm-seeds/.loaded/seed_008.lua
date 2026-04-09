
-- 8: __eq/__lt/__le triggers allocation during GC
local function alloc_meta(op)
  local t = {}
  local mt = {}
  mt["__" .. op] = function(a, b)
    local tmp = {}
    for i = 1, 20 do tmp[i] = string.rep("z", 64) end
    collectgarbage("step", 1)
    return true
  end
  mt.__gc = function() collectgarbage("step", 0) end
  setmetatable(t, mt)
  return t
end
local a = alloc_meta("eq")
local b = setmetatable({}, getmetatable(a))
local c = alloc_meta("lt")
local d = setmetatable({}, getmetatable(c))
collectgarbage("collect")
local _ = (a == b)
collectgarbage("step", 1)
local _ = (c < d)
collectgarbage("collect")
local _ = (c <= d)
collectgarbage("collect")
