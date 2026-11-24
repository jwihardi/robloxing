--!strict
local Salvage = require("./src/Salvage")
assert(Salvage.remaining(5, 2) == 3, "collecting two of five leaves three")
assert(Salvage.remaining(2, 4) == 0, "remaining salvage cannot be negative")
print("Salvage checks passed")
