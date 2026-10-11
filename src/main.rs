mod raylib;

use crate::raylib::*;

enum GameScreen {
    Logo,
    Title,
    Gameplay,
    Ending,
}

fn main() {
    const WIDTH: i32 = 800;
    const HEIGHT: i32 = 450;

    InitWindow(WIDTH, HEIGHT, "PROJECT: BLOCKS GAME");

    let mut screen = GameScreen::Logo;
    let mut frames_counter = 0;
    let mut game_result = -1;
    let mut game_paused = false;

    SetTargetFPS(60);

    while !WindowShouldClose() {
        match screen {
            GameScreen::Logo => {
                frames_counter += 1;
                if frames_counter > 180 {
                    screen = GameScreen::Title;
                    frames_counter = 0;
                }
            }
            GameScreen::Title => {
                frames_counter += 1;
                if IsKeyDown(KEY_ENTER) {
                    screen = GameScreen::Gameplay;
                }
            }
            GameScreen::Gameplay => {
                if !game_paused {
                    // game play logic
                }
                if IsKeyPressed(KEY_ENTER) {
                    screen = GameScreen::Ending;
                }
            }
            GameScreen::Ending => {
                frames_counter += 1;
                if IsKeyPressed(KEY_ENTER) {
                    screen = GameScreen::Title;
                }
            }
        }

        BeginDrawing();
            ClearBackground(RAYWHITE);
            match screen {
                GameScreen::Logo => {
                    DrawText("LOGO SCREEN", 20, 20, 40, LIGHTGRAY);
                    DrawText("WAIT for 3 SECONDS...", 290, 220, 20, GRAY);
                }
                GameScreen::Title => {
                    DrawRectangle(0, 0, WIDTH, HEIGHT, GREEN);
                    DrawText("TITLE SCREEN", 20, 20, 40, DARKGREEN);
                    DrawText("PRESS ENTER or TAP to JUMP to GAMEPLAY SCREEN", 120, 220, 20, DARKGREEN);
                }
                GameScreen::Gameplay => {
                    DrawRectangle(0, 0, WIDTH, HEIGHT, PURPLE);
                    DrawText("GAMEPLAY SCREEN", 20, 20, 40, MAROON);
                    DrawText("PRESS ENTER or TAP to JUMP to ENDING SCREEN", 130, 220, 20, MAROON);
                }
                GameScreen::Ending => {
                    DrawRectangle(0, 0, WIDTH, HEIGHT, BLUE);
                    DrawText("ENDING SCREEN", 20, 20, 40, DARKBLUE);
                    DrawText("PRESS ENTER or TAP to RETURN to TITLE SCREEN", 120, 220, 20, DARKBLUE);
                }
            }
        EndDrawing();
    }

    CloseWindow();
}
