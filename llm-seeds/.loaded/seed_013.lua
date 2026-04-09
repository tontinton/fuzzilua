
-- 13: loadstring inside __gc (allocation-heavy during finalization)
local results = {}
local function make_loader(idx)
  local t = newproxy(true)
  local mt = getmetatable(t)
  mt.__gc = function(self)
    local code = "local t = {} for i=1," .. tostring(idx * 5) .. " do t[i]=i end return t[1]"
    local fn = loadstring(code)
    if fn then
      local ok, val = pcall(fn)
      if ok then results[idx] = val end
    end
    collectgarbage("step", 1)
  end
  return t
end
for i = 1, 12 do make_loader(i) end
collectgarbage("collect")
collectgarbage("collect")
local sum = 0
for k, v in pairs(results) do sum = sum + v end
local _ = sum
