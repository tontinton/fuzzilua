
-- 6: Table rehash at boundary (8 array -> 9 triggers rehash), GC during insert
local t = {}
for i = 1, 8 do t[i] = string.rep("a", 64) end
collectgarbage("collect")
local pressure = {}
for i = 1, 100 do pressure[i] = {string.rep("p", 128)} end
collectgarbage("step", 0)
t[9] = string.rep("b", 256)
collectgarbage("step", 1)
t["hash1"] = string.rep("c", 128)
t["hash2"] = string.rep("d", 128)
collectgarbage("collect")
for i = 1, 12 do
  t[i + 9] = {}
  collectgarbage("step", 0)
end
pressure = nil
collectgarbage("collect")
local _ = t[9] .. t[1]
