#![no_std]
#![forbid(unsafe_code)]
//! # embassy-st7789v-graphics
//!
//! Couche graphique 2D `no_std` pour écrans TFT LCD ST7789V 240×320,
//! construite au-dessus de [`embassy-st7789v`](https://docs.rs/embassy-st7789v).
//!
//! ## Rôle exact de ce crate
//!
//! Le driver `embassy-st7789v` fournit déjà :
//! - `draw_pixel()`, `draw_hline()`, `draw_vline()`
//! - `draw_rect()`, `fill_rect()`, `fill_screen()`
//! - `draw_char()`, `draw_str()`, `draw_i16()`, `draw_u32()`, `draw_f32()`
//! - `draw_char_scaled()`, `draw_str_scaled()`
//! - `draw_bitmap()`
//! - `set_orientation()`, `set_invert()`
//!
//! Ce crate **ne duplique rien**. Il ajoute uniquement les primitives
//! géométriques que le driver ne propose pas :
//!
//! | Fonction          | Algorithme                      |
//! |-------------------|---------------------------------|
//! | [`line()`]          | Bresenham integer-only          |
//! | [`circle()`]        | Midpoint integer-only           |
//! | [`fill_circle()`]   | Midpoint + hlines               |
//! | [`triangle()`]      | 3 appels à [`line()`]           |
//! | [`fill_triangle`] | Scanline integer-only           |
//! | [`ellipse`]       | Midpoint généralisé             |
//! | [`bezier_quad`]   | De Casteljau integer-only       |
//!
//! ## Architecture
//!
//! ```text
//! ┌──────────────────────────────────────────────────┐
//! │               Votre application                  │
//! │  line() / circle() / triangle() …               │
//! │  ecran.draw_str() / ecran.draw_f32() …          │  ← driver direct pour le texte
//! └──────────┬───────────────────────────────────────┘
//!            │ &mut Graphics
//! ┌──────────▼──────────────────┐
//! │  Graphics (ce crate)        │
//! │  clipping · pixel() async   │
//! └──────────┬──────────────────┘
//!            │ draw_pixel() async
//! ┌──────────▼──────────────────────────────────────┐
//! │     embassy-st7789v (driver)                    │
//! │  framebuffer SPI · RAMWR · fill_rect() …       │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! ## Patron de borrow
//!
//! `Graphics` tient un `&mut St7789v` pour toute sa durée de vie.
//! Pour appeler les méthodes du driver directement (texte, remplissage,
//! orientation…), `gfx` doit être sorti de portée au préalable.
//!
//! ```rust,no_run
//! loop {
//!     ecran.fill_screen(Color::BLACK).await.unwrap();
//!     {
//!         let mut gfx = Graphics::new(&mut ecran);
//!         line(&mut gfx, 0, 0, 239, 319, Color::WHITE).await;
//!         circle(&mut gfx, 120, 160, 60, Color::CYAN).await;
//!     } // ← borrow libéré
//!     ecran.draw_str(8, 10, b"BONJOUR", Color::YELLOW, Color::BLACK).await.unwrap();
//! }
//! ```
//!
//! ## Note sur les erreurs SPI
//!
//! Les fonctions de ce crate ignorent silencieusement les erreurs SPI
//! (comme le fait également `embassy-ssd1306-graphics` pour le bus I2C).
//! Si votre application requiert une gestion d'erreur fine, utilisez
//! directement `ecran.draw_pixel()`.

use embassy_st7789v::{Color, NoPin, St7789v, SCREEN_H, SCREEN_W};
use embedded_hal::digital::OutputPin;
use embedded_hal_async::spi::SpiDevice;

// ─────────────────────────────────────────────────────────────────────────────
// Contexte graphique
// ─────────────────────────────────────────────────────────────────────────────

/// Contexte graphique pour le ST7789V 240×320.
///
/// Wraps un `&mut St7789v<SPI, DC, RST>` pour :
/// - centraliser le **clipping** des coordonnées signées
/// - fournir un `pixel()` async en `i32` aux algorithmes Bresenham / midpoint
///
/// Le driver reste propriétaire du bus SPI et des broches.
///
/// # Construction
///
/// ```rust,no_run
/// let mut gfx = Graphics::new(&mut ecran);
/// line(&mut gfx, 0, 0, 239, 319, Color::RED).await;
/// ```
///
/// Utilisez [`Graphics::new_no_rst`] si votre `St7789v` a été construit
/// avec [`St7789v::new_no_rst`].
pub struct Graphics<'a, SPI, DC, RST = NoPin>
where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    display: &'a mut St7789v<SPI, DC, RST>,
}

// ── Constructeur sans RST ─────────────────────────────────────────────────────

impl<'a, SPI, DC> Graphics<'a, SPI, DC, NoPin>
where
    SPI: SpiDevice,
    DC: OutputPin,
{
    /// Crée un contexte graphique pour un `St7789v` sans broche RST.
    #[inline]
    pub fn new_no_rst(display: &'a mut St7789v<SPI, DC, NoPin>) -> Self {
        Self { display }
    }
}

// ── Constructeur avec RST ─────────────────────────────────────────────────────

impl<'a, SPI, DC, RST> Graphics<'a, SPI, DC, RST>
where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    /// Crée un contexte graphique pour un `St7789v` avec broche RST matérielle.
    #[inline]
    pub fn new(display: &'a mut St7789v<SPI, DC, RST>) -> Self {
        Self { display }
    }

    /// Dessine un pixel avec clipping automatique.
    ///
    /// Les coordonnées négatives ou hors de `[0, 240[` × `[0, 320[`
    /// sont silencieusement ignorées : aucun panic, aucun wrapping.
    ///
    /// Les erreurs SPI sont ignorées (cohérent avec l'usage dans les
    /// algorithmes géométriques qui tracent des milliers de pixels).
    #[inline(always)]
    pub async fn pixel(&mut self, x: i32, y: i32, color: Color) {
        if x >= 0 && y >= 0 && x < SCREEN_W as i32 && y < SCREEN_H as i32 {
            let _ = self.display.draw_pixel(x as u16, y as u16, color).await;
        }
    }

    /// Dessine une ligne horizontale interne (utilisée par les algorithmes de remplissage).
    ///
    /// Effectue le clipping vertical puis délègue à `fill_rect`.
    #[inline]
    async fn hline(&mut self, x0: i32, x1: i32, y: i32, color: Color) {
        if y < 0 || y >= SCREEN_H as i32 {
            return;
        }
        let xa = x0.max(0).min(SCREEN_W as i32 - 1) as u16;
        let xb = x1.max(0).min(SCREEN_W as i32 - 1) as u16;
        let (xa, xb) = if xa <= xb { (xa, xb) } else { (xb, xa) };
        let _ = self
            .display
            .fill_rect(xa, y as u16, xb, y as u16, color)
            .await;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ligne Bresenham
// ─────────────────────────────────────────────────────────────────────────────

/// Trace une ligne entre `(x0, y0)` et `(x1, y1)`.
///
/// **Algorithme :** Bresenham integer-only.  
/// Zéro division flottante, zéro multiplication, sûr sur tout MCU sans FPU.
///
/// # Exemple
///
/// ```rust,no_run
/// line(&mut gfx, 0, 0, 239, 319, Color::WHITE).await;  // diagonale complète
/// line(&mut gfx, 0, 0, 239, 319, Color::BLACK).await;  // efface la diagonale
/// ```
pub async fn line<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;

    loop {
        gfx.pixel(x0, y0, color).await;
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cercle midpoint
// ─────────────────────────────────────────────────────────────────────────────

/// Trace le **contour** d'un cercle.
///
/// **Algorithme :** midpoint circle integer-only.  
/// Exploite la symétrie 8-octants : chaque itération dessine 8 pixels
/// symétriques, ce qui minimise le nombre d'appels à `pixel()`.
///
/// # Paramètres
///
/// - `(cx, cy)` : centre
/// - `r` : rayon en pixels
/// - `color` : couleur du contour
///
/// # Exemple
///
/// ```rust,no_run
/// circle(&mut gfx, 120, 160, 60, Color::CYAN).await;
/// ```
pub async fn circle<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    cx: i32,
    cy: i32,
    r: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    if r <= 0 {
        gfx.pixel(cx, cy, color).await;
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 0;

    while x >= y {
        gfx.pixel(cx + x, cy + y, color).await;
        gfx.pixel(cx + y, cy + x, color).await;
        gfx.pixel(cx - y, cy + x, color).await;
        gfx.pixel(cx - x, cy + y, color).await;
        gfx.pixel(cx - x, cy - y, color).await;
        gfx.pixel(cx - y, cy - x, color).await;
        gfx.pixel(cx + y, cy - x, color).await;
        gfx.pixel(cx + x, cy - y, color).await;

        y += 1;
        if err <= 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

/// **Remplit** un cercle (disque plein).
///
/// Utilise le même algorithme midpoint, mais dessine des lignes
/// horizontales entre les points symétriques à chaque rangée.
/// Beaucoup plus rapide qu'un appel pixel par pixel, car `fill_rect`
/// envoie les données en bloc via SPI.
///
/// # Exemple
///
/// ```rust,no_run
/// fill_circle(&mut gfx, 120, 160, 50, Color::BLUE).await;
/// ```
pub async fn fill_circle<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    cx: i32,
    cy: i32,
    r: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    if r <= 0 {
        gfx.pixel(cx, cy, color).await;
        return;
    }
    let mut x = r;
    let mut y = 0;
    let mut err = 0;

    while x >= y {
        gfx.hline(cx - x, cx + x, cy + y, color).await;
        gfx.hline(cx - x, cx + x, cy - y, color).await;
        gfx.hline(cx - y, cx + y, cy + x, color).await;
        gfx.hline(cx - y, cx + y, cy - x, color).await;

        y += 1;
        if err <= 0 {
            err += 2 * y + 1;
        } else {
            x -= 1;
            err += 2 * (y - x) + 1;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Triangle
// ─────────────────────────────────────────────────────────────────────────────

/// Trace le **contour** d'un triangle défini par trois sommets.
///
/// Implémenté comme trois appels à [`line()`], aucune logique propre.
///
/// # Exemple
///
/// ```rust,no_run
/// triangle(&mut gfx, 120, 10, 20, 310, 220, 310, Color::GREEN).await;
/// ```
pub async fn triangle<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    line(gfx, x0, y0, x1, y1, color).await;
    line(gfx, x1, y1, x2, y2, color).await;
    line(gfx, x2, y2, x0, y0, color).await;
}

// ─────────────────────────────────────────────────────────────────────────────
// Ellipse midpoint généralisé
// ─────────────────────────────────────────────────────────────────────────────

/// Trace le **contour** d'une ellipse.
///
/// **Algorithme :** midpoint ellipse integer-only (Bresenham généralisé).
/// Deux phases : région 1 (pente < -1) puis région 2 (pente > -1).
///
/// # Paramètres
///
/// - `(cx, cy)` : centre
/// - `rx` : demi-axe horizontal en pixels
/// - `ry` : demi-axe vertical en pixels
/// - `color` : couleur du contour
///
/// # Exemple
///
/// ```rust,no_run
/// ellipse(&mut gfx, 120, 160, 100, 60, Color::MAGENTA).await; // ellipse large
/// ellipse(&mut gfx, 120, 160, 40, 40, Color::WHITE).await;    // cercle (rx == ry)
/// ```
pub async fn ellipse<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    cx: i32,
    cy: i32,
    rx: i32,
    ry: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    if rx <= 0 || ry <= 0 {
        gfx.pixel(cx, cy, color).await;
        return;
    }

    let rx2 = rx * rx;
    let ry2 = ry * ry;

    let mut x = 0i32;
    let mut y = ry;

    // Région 1
    let mut d1 = ry2 - rx2 * ry + rx2 / 4;
    let mut dx = 2 * ry2 * x;
    let mut dy = 2 * rx2 * y;

    while dx < dy {
        gfx.pixel(cx + x, cy + y, color).await;
        gfx.pixel(cx - x, cy + y, color).await;
        gfx.pixel(cx + x, cy - y, color).await;
        gfx.pixel(cx - x, cy - y, color).await;

        x += 1;
        dx += 2 * ry2;
        if d1 < 0 {
            d1 += dx + ry2;
        } else {
            y -= 1;
            dy -= 2 * rx2;
            d1 += dx - dy + ry2;
        }
    }

    // Région 2
    let mut d2 = ry2 * (x * x + x) + rx2 * (y * y - 2 * y + 1) - rx2 * ry2 + rx2;

    while y >= 0 {
        gfx.pixel(cx + x, cy + y, color).await;
        gfx.pixel(cx - x, cy + y, color).await;
        gfx.pixel(cx + x, cy - y, color).await;
        gfx.pixel(cx - x, cy - y, color).await;

        y -= 1;
        dy -= 2 * rx2;
        if d2 > 0 {
            d2 += rx2 - dy;
        } else {
            x += 1;
            dx += 2 * ry2;
            d2 += dx - dy + rx2;
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Courbe de Bézier quadratique De Casteljau
// ─────────────────────────────────────────────────────────────────────────────

/// Trace une **courbe de Bézier quadratique** (3 points de contrôle).
///
/// **Algorithme :** De Casteljau integer-only avec subdivision fixe.
/// `steps` contrôle la finesse du tracé (16–32 suffisent pour 240×320).
///
/// Les interpolations sont effectuées en entiers avec précision ×1024
/// pour éviter tout calcul flottant.
///
/// # Paramètres
///
/// - `(x0, y0)` : point de départ
/// - `(x1, y1)` : point de contrôle
/// - `(x2, y2)` : point d'arrivée
/// - `steps` : nombre de segments (recommandé : 16 à 32)
/// - `color` : couleur de la courbe
///
/// # Exemple
///
/// ```rust,no_run
/// // Arche : point de contrôle en haut au centre
/// bezier_quad(&mut gfx, 10, 280, 120, 40, 230, 280, 24, Color::ORANGE).await;
/// ```
pub async fn bezier_quad<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    x2: i32,
    y2: i32,
    steps: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    if steps <= 0 {
        return;
    }

    let mut px = x0;
    let mut py = y0;

    for i in 1..=steps {
        // t = i / steps en virgule fixe ×1024
        let t = (i * 1024) / steps; // t  ∈ [0, 1024]
        let t1 = 1024 - t; // 1-t

        // B(t) = (1-t)²·P0 + 2(1-t)t·P1 + t²·P2  (tout ×1024²)
        let nx = (t1 * t1 * x0 + 2 * t1 * t * x1 + t * t * x2) / (1024 * 1024);
        let ny = (t1 * t1 * y0 + 2 * t1 * t * y1 + t * t * y2) / (1024 * 1024);

        line(gfx, px, py, nx, ny, color).await;
        px = nx;
        py = ny;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Triangle plein — scanline
// ─────────────────────────────────────────────────────────────────────────────

/// **Remplit** un triangle défini par trois sommets.
///
/// **Algorithme :** scanline : tri des sommets par Y, puis
/// interpolation linéaire integer-only des bords gauche/droit
/// à chaque rangée horizontale. Utilise `fill_rect` pour envoyer
/// chaque ligne en un seul transfert SPI.
///
/// # Exemple
///
/// ```rust,no_run
/// fill_triangle(&mut gfx, 120, 10, 20, 310, 220, 310, Color::YELLOW).await;
/// ```
pub async fn fill_triangle<SPI, DC, RST>(
    gfx: &mut Graphics<'_, SPI, DC, RST>,
    x0: i32,
    mut y0: i32,
    x1: i32,
    mut y1: i32,
    x2: i32,
    mut y2: i32,
    color: Color,
) where
    SPI: SpiDevice,
    DC: OutputPin,
    RST: OutputPin,
{
    // Tri des sommets par Y croissant (bubble sort sur 3 éléments)
    let (mut x0, mut x1, mut x2) = (x0, x1, x2);
    if y0 > y1 {
        core::mem::swap(&mut y0, &mut y1);
        core::mem::swap(&mut x0, &mut x1);
    }
    if y1 > y2 {
        core::mem::swap(&mut y1, &mut y2);
        core::mem::swap(&mut x1, &mut x2);
    }
    if y0 > y1 {
        core::mem::swap(&mut y0, &mut y1);
        core::mem::swap(&mut x0, &mut x1);
    }

    let total_h = y2 - y0;
    if total_h == 0 {
        // Triangle dégénéré : tracer une seule ligne
        let xmin = x0.min(x1).min(x2);
        let xmax = x0.max(x1).max(x2);
        gfx.hline(xmin, xmax, y0, color).await;
        return;
    }

    let upper_h = y1 - y0;
    let lower_h = y2 - y1;

    // Moitié supérieure : y0 → y1
    for y in y0..=y1 {
        let dy = y - y0;
        let xa = x0 + (x2 - x0) * dy / total_h;
        let xb = if upper_h == 0 {
            x1
        } else {
            x0 + (x1 - x0) * dy / upper_h
        };
        gfx.hline(xa, xb, y, color).await;
    }

    // Moitié inférieure : y1 → y2
    for y in y1..=y2 {
        let dy = y - y0;
        let xa = x0 + (x2 - x0) * dy / total_h;
        let xb = if lower_h == 0 {
            x1
        } else {
            x1 + (x2 - x1) * (y - y1) / lower_h
        };
        gfx.hline(xa, xb, y, color).await;
    }
}