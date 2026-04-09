
-- 11: setmetatable during __newindex while GC is propagating
local root = {}
local child = {}
local gc_trigger = newproxy(true)
getmetatable(gc_trigger).__gc = function()
  local mt = {__newindex = function(self, k, v)
    local new_mt = {__index = function() return "replaced" end}
    setmetatable(self, new_mt)
    rawset(self, k, v)
    collectgarbage("step", 1)
  end}
  setmetatable(child, mt)
  child.injected = true
  collectgarbage("step", 0)
end
gc_trigger = nil
for i = 1, 20 do root[i] = string.rep("m", 64) end
collectgarbage("collect")
child.after_gc = "test"
collectgarbage("collect")
local _ = child.injected
local _ = child.after_gc
