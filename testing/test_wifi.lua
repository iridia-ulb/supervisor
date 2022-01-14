--[[ This function is executed every time you press the 'execute' button ]]
function init()
end

function step()
   data = {
      vec = vector3(-1.111, 2.222, -3.333),
   }
   robot.simple_radios.wifi.send(data)
   for index, message in pairs(robot.simple_radios.wifi.recv) do
      log(tostring(message.vec))
   end
end

function reset()
end

function destroy()
end
