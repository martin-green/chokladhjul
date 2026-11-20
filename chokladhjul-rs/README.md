# Chokladhjul (Rust + OpenGL)

Chokladhjulet från `www/` omskrivet i Rust med OpenGL 3.3 och bättre animeringar.
Allt (bilder och ljud) är inbakat i programfilen, så det räcker att kopiera
`chokladhjul.exe`.

## Styrning

| Tangent / knapp       | Funktion                                   |
|-----------------------|--------------------------------------------|
| Mellanslag, musknapp  | Snurra (en ny knapptryckning under snurren ger mer fart, som i originalet) |
| F11 eller F           | Växla helskärm                             |
| Esc                   | Lämna helskärm, eller avsluta i fönsterläge |

## Flaggor

```
chokladhjul [--windowed] [--classic] [--no-sound] [--seed <n>] [--capture <dir>] [--bench]
```

- `--windowed` startar i fönster (standard är helskärm utan muspekare).
- `--classic` använder det klassiska vinsthjulet (`prizewheel.png`: 1 stjärnvinst,
  2 förstapris, resten nitlotter). Standard är 50/50-hjulet (`prizewheel2.png`),
  precis som `fiftyFiftyMode = true` i webbversionen.
- `--capture <dir>` renderar en snurr till PNG-filer och avslutar (för att kontrollera grafiken).
- `--bench` renderar så fort det går under några snurrar och skriver ut bildtiderna.

## Animeringar

- **Riktig fysik:** pinnar på hjulets kant slår mot en fjädrande pekare. Hjulet bromsas
  tick för tick på slutet och kan rulla tillbaka om det inte orkar över en pinne.
  Tickljudet spelas vid varje pinnträff, med volym efter hur hårt pinnen slår.
- **Rörelseoskärpa** på hjulet och pilen, så att det inte ser ut att hacka eller
  snurra baklänges när det går fort.
- **Strålkastare** på vinnande sektor och vinsttyp, plus en mjuk dimning av resten.
- Resultattexten poppar fram när hjulet stannar. Stjärnvinst får guldtext,
  fanfar, konfetti och ljusstrålar.
- Ljusstrålar i bakgrunden och skuggor under hjulen.

Utfallet är fortfarande helt slumpmässigt. Testerna kontrollerar att alla 24 nummer
kommer upp lika ofta.

## Bygga

Kräver Rust (stable).

```
cargo build --release
```

Programmet hamnar i `target\release\chokladhjul.exe`. Tester: `cargo test --release`.

## Prestanda

Målet är 60 fps på en i7-3610QM med GeForce GT 630M:

- OpenGL 3.3 core. Både GT 630M och den inbyggda Intel HD 4000 klarar det.
- Vsync ger 60 fps. Om vsync inte fungerar tar en enkel bildbegränsare över.
- Programfilen exporterar `NvOptimusEnablement`, så att Optimus-datorer kör på
  GeForce-kortet i stället för Intel-grafiken.
- Rörelseoskärpan tar fler texturprov bara när hjulet snurrar fort, och använder
  lägre mipmap-nivåer då.

## Kod

| Fil              | Innehåll                                                        |
|------------------|-----------------------------------------------------------------|
| `src/main.rs`    | Fönster, OpenGL-kontext, tangenter, `--capture` och `--bench`   |
| `src/game.rs`    | Spellogik, animeringar, partiklar och rendering                 |
| `src/physics.rs` | Hjul, pinnar, pekare och pilen, med tester                      |
| `src/shaders.rs` | GLSL-shaders (bakgrund, hjul med oskärpa, SDF-text)             |
| `src/textures.rs`| Bakar kant och pinnar på hjulet, ritar pekare, glans och gnistor |
| `src/text.rs`    | Avståndsfältstext (typsnitt från systemet, t.ex. Arial Black)   |
| `src/audio.rs`   | Ljud via rodio                                                  |
