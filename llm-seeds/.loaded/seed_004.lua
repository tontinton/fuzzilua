
-- 4: Coroutine yields inside __newindex, force GC before resume
local co
local trap = {}
local mt = {
  __newindex = function(self, k, v)
    rawset(self, k, v)
    if co and coroutine.status(co) == "running" then
      coroutine.yield(k)
    end
  end
}
setmetatable(trap, mt)
co = coroutine.create(function()
  for i = 1, 20 do
    trap[i] = string.rep("y", 128)
  end
  return trap
end)
for i = 1, 20 do
  local ok, val = coroutine.resume(co)
  if not ok then break end
  collectgarbage("collect")
  local filler = {}
  for j = 1, 50 do filler[j] = {} end
  filler = nil
  collectgarbage("collect")
end
local _ = trap[1]
