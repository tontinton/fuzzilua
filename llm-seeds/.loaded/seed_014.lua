
-- 14: rawset inside __newindex inside __gc chain
local log = {}
local inner = setmetatable({}, {
  __newindex = function(self, k, v)
    rawset(self, k, v)
    rawset(log, #log + 1, k)
    collectgarbage("step", 0)
    local tmp = string.rep("n", 64)
    rawset(self, k .. "_copy", tmp)
  end
})
local function make_gc_writer(tbl, prefix)
  local t = newproxy(true)
  getmetatable(t).__gc = function(self)
    for i = 1, 5 do
      tbl[prefix .. i] = string.rep(prefix, 16)
    end
    collectgarbage("step", 1)
  end
  return t
end
local objs = {}
for i = 1, 10 do objs[i] = make_gc_writer(inner, "gc" .. i .. "_") end
objs = nil
collectgarbage("collect")
collectgarbage("collect")
local _ = #log
for k, v in pairs(inner) do local _ = k .. tostring(v) end
