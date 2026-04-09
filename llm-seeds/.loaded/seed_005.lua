
-- 5: String interning pressure
local strs = {}
local base = "fuzz_gc_test_string_"
for i = 1, 200 do
  strs[i] = base .. tostring(i % 20)
end
collectgarbage("collect")
local weak_strs = setmetatable({}, {__mode = "v"})
for i = 1, 200 do
  local s = base .. tostring(i % 20)
  weak_strs[i] = s
end
strs = nil
collectgarbage("collect")
collectgarbage("collect")
local alive = 0
for i = 1, 200 do
  if weak_strs[i] then
    alive = alive + string.len(weak_strs[i])
  end
end
local _ = base .. tostring(alive)
