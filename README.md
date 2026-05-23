# embassy-st7789v-graphics

Couche graphique 2D `no_std` pour écrans TFT LCD **ST7789V 240×320**,
construite au-dessus de [`embassy-st7789v`](https://crates.io/crates/embassy-st7789v).

## Fonctionnalités ajoutées

| Fonction          | Algorithme                  |
|-------------------|-----------------------------|
| `line`            | Bresenham integer-only      |
| `circle`          | Midpoint integer-only       |
| `fill_circle`     | Midpoint + hlines SPI batch |
| `triangle`        | 3 × `line`                  |
| `fill_triangle`   | Scanline integer-only       |
| `ellipse`         | Midpoint généralisé         |
| `bezier_quad`     | De Casteljau integer-only   |

Toutes les primitives :
- Zéro allocation (`no_std`, pas de `Vec`)
- Zéro flottant (sûr sur MCU sans FPU)
- Clipping automatique sur 240×320
- Couleurs `Color` RGB565 (même type que le driver)

## Démarrage rapide

```toml
[dependencies]
embassy-st7789v          = "0.1"
embassy-st7789v-graphics = "0.1"
```

```rust
use embassy_st7789v::{Color, St7789v};
use embassy_st7789v_graphics::{Graphics, circle, fill_triangle, line};

// Init du driver
let mut ecran = St7789v::new(spi_device, broche_dc, broche_rst);
ecran.init().await.unwrap();
ecran.fill_screen(Color::BLACK).await.unwrap();

// Primitives graphiques
{
    let mut gfx = Graphics::new(&mut ecran);
    line(&mut gfx, 0, 0, 239, 319, Color::WHITE).await;
    circle(&mut gfx, 120, 160, 60, Color::CYAN).await;
    fill_triangle(&mut gfx, 120, 10, 20, 310, 220, 310, Color::YELLOW).await;
} // ← borrow libéré

// Texte via le driver directement
ecran.draw_str(8, 8, b"BONJOUR", Color::WHITE, Color::BLACK).await.unwrap();
```

## Sans broche RST

```rust
let mut ecran = St7789v::new_no_rst(spi_device, broche_dc);
// ...
let mut gfx = Graphics::new_no_rst(&mut ecran);
```

## Licence

GPL-2.0-or-later — Copyright (C) 2026 Jorge Andre Castro