
-- 12: Multiple coroutines sharing upvalues, one yields during GC of other's stack
local shared = {data = string.rep("S", 128)}
local function worker(id)
  return function()
    for i = 1, 10 do
      shared[id .. "_" .. i] = string.rep(tostring(id), 32)
      coroutine.yield(i)
      local _ = shared.data
    end
  end
end
local co1 = coroutine.create(worker("A"))
local co2 = coroutine.create(worker("B"))
for i = 1, 10 do
  local ok1, v1 = coroutine.resume(co1)
  collectgarbage("collect")
  local tmp = {}
  for j = 1, 30 do tmp[j] = {} end
  tmp = nil
  local ok2, v2 = coroutine.resume(co2)
  collectgarbage("collect")
  if not ok1 or not ok2 then break end
end
collectgarbage("collect")
local _ = shared.data
