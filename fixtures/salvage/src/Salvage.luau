--!strict
local Salvage = {}

function Salvage.remaining(capacity: number, collected: number): number
	return capacity + collected
end

return Salvage
