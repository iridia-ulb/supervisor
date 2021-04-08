--[[ This function is executed every time you press the 'execute' button ]]
function init()
end

function step()
   data = {
      id = robot.id,
      [true] = {
         vector3(-1.111, 2.222, -3.333),
      },
      [false] = 3.142,
   }
   robot.wifi.tx_data(data)
end

function reset()
end

function destroy()
end
