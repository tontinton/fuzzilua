
-- 10: __gc finalizer calls collectgarbage("collect") recursively
local depth = 0
local function make_recursive_gc(n)
  local t = newproxy(true)
  local mt = getmetatable(t)
  mt.__gc = function(self)
    depth = depth + 1
    if depth < 6 then
      local inner = newproxy(true)
      getmetatable(inner).__gc = function(s)
        depth = depth + 1
        collectgarbage("collect")
      end
      inner = nil
      collectgarbage("collect")
    end
    depth = depth - 1
  end
  return t
end
local objs = {}
for i = 1, 15 do objs[i] = make_recursive_gc(i) end
objs = nil
collectgarbage("collect")
collectgarbage("collect")
collectgarbage("collect")
local _ = depth
