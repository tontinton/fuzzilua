
-- 9: Weak-keyed table with string keys, collect during iteration
local wk = setmetatable({}, {__mode = "k"})
local refs = {}
for i = 1, 50 do
  local k = tostring(i) .. string.rep("k", 32)
  wk[k] = i
  if i % 3 == 0 then refs[#refs + 1] = k end
end
collectgarbage("collect")
local count = 0
for k, v in pairs(wk) do
  count = count + 1
  if count % 5 == 0 then
    collectgarbage("collect")
  end
  local _ = string.len(k) + v
end
refs = nil
collectgarbage("collect")
for k, v in pairs(wk) do
  local _ = k .. tostring(v)
end
