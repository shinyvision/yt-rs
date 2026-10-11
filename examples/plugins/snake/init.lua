-- Snake, as a page in the sidebar.
--
-- Shows: drawing with Cairo (ui.drawing_area), animation with on_tick, key
-- input with a GTK event controller (any GTK type works through ui.new).

local CELL, COLS, ROWS = 24, 32, 20
local STEP = 0.11 -- seconds per move

local function new_game()
  return {
    snake = { { x = 10, y = 10 }, { x = 9, y = 10 }, { x = 8, y = 10 } },
    dir = { x = 1, y = 0 },
    next_dir = { x = 1, y = 0 },
    food = { x = 20, y = 10 },
    score = 0,
    over = false,
    last = nil,
  }
end

local function place_food(game)
  while true do
    local food = { x = math.random(0, COLS - 1), y = math.random(0, ROWS - 1) }
    local free = true
    for _, part in ipairs(game.snake) do
      if part.x == food.x and part.y == food.y then free = false end
    end
    if free then game.food = food return end
  end
end

local function step(game)
  game.dir = game.next_dir
  local head = { x = game.snake[1].x + game.dir.x, y = game.snake[1].y + game.dir.y }
  if head.x < 0 or head.y < 0 or head.x >= COLS or head.y >= ROWS then
    game.over = true
    return
  end
  for _, part in ipairs(game.snake) do
    if part.x == head.x and part.y == head.y then game.over = true return end
  end
  table.insert(game.snake, 1, head)
  if head.x == game.food.x and head.y == game.food.y then
    game.score = game.score + 1
    place_food(game)
  else
    table.remove(game.snake)
  end
end

local function draw(game, cr, width, height)
  local ox, oy = (width - COLS * CELL) / 2, (height - ROWS * CELL) / 2
  cr:set_color("#181818")
  cr:rectangle(ox, oy, COLS * CELL, ROWS * CELL)
  cr:fill()
  cr:set_color("#ff0033")
  cr:arc(ox + (game.food.x + 0.5) * CELL, oy + (game.food.y + 0.5) * CELL, CELL * 0.35, 0, 2 * math.pi)
  cr:fill()
  for i, part in ipairs(game.snake) do
    cr:set_source_rgba(1, 1, 1, i == 1 and 1 or 0.75)
    cr:rectangle(ox + part.x * CELL + 2, oy + part.y * CELL + 2, CELL - 4, CELL - 4)
    cr:fill()
  end
  cr:set_color("#f1f1f1")
  cr:select_font_face("Sans", true)
  cr:set_font_size(18)
  cr:move_to(ox, oy - 12)
  cr:show_text(game.over and ("Game over. Score " .. game.score .. ". Press space") or ("Score " .. game.score))
end

-- Arrow key values from gdk/gdkkeysyms.h.
local KEYS = { [65361] = { x = -1, y = 0 }, [65362] = { x = 0, y = -1 }, [65363] = { x = 1, y = 0 }, [65364] = { x = 0, y = 1 } }

app.page {
  name = "snake",
  title = "Snake",
  keep = false,
  build = function()
    local game = new_game()
    local area = ui.drawing_area(function(cr, width, height) draw(game, cr, width, height) end)
    area.hexpand, area.vexpand, area.focusable = true, true, true

    local keys = ui.new("GtkEventControllerKey")
    keys:on("key-pressed", function(_, keyval)
      local dir = KEYS[keyval]
      if dir and (dir.x ~= -game.dir.x or dir.y ~= -game.dir.y) then
        game.next_dir = dir
        return true
      end
      if keyval == 32 and game.over then -- space
        game = new_game()
        return true
      end
      return false
    end)
    area:add_controller(keys)

    area:on_tick(function(time)
      game.last = game.last or time
      if not game.over and time - game.last >= STEP then
        game.last = time
        step(game)
        area:queue_draw()
      end
      return true
    end)
    area:on("map", function(self) self:grab_focus() end)
    return area
  end,
}

app.nav { id = "snake", title = "Snake", icon = "applications-games-symbolic", page = "snake", section = "library", order = 60 }
