
-- 3: Deep metatable chain with __index triggering GC via table creation
local function build_chain(depth)
  if depth == 0 then return {leaf = true} end
  local inner = build_chain(depth - 1)
  local t = {}
  local mt = {
    __index = function(self, key)
      local tmp = {}
      for i = 1, 10 do tmp[i] = string.rep("x", 64) end
      collectgarbage("step", 2)
      return inner[key]
    end
  }
  setmetatable(t, mt)
  return t
end
local chain = build_chain(8)
collectgarbage("collect")
for i = 1, 10 do
  local _ = chain.leaf
  collectgarbage("step", 1)
end
collectgarbage("collect")
local _ = chain.leaf
