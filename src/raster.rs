//! Rasterizer

use POLY_SUBPIXEL_SHIFT;
use POLY_SUBPIXEL_SCALE;

use clip::Clip;
use scan::ScanlineU8;
use cell::RasterizerCell;
use path_storage::PathCommand;
use render::RendererPrimatives;

use Rasterize;
use VertexSource;
use PixfmtFunc;
use Pixel;

use std::cmp::min;
use std::cmp::max;

struct RasConvInt {
}
impl RasConvInt {
    pub fn upscale(v: f64) -> i64 {
        (v * POLY_SUBPIXEL_SCALE as f64).round() as i64
    }
    //pub fn downscale(v: i64) -> i64 {
    //    v
    //}
}

/// Winding / Filling Rule
///
/// See (Non-Zero Filling Rule)[https://en.wikipedia.org/wiki/Nonzero-rule] and
/// (Even-Odd Filling)[https://en.wikipedia.org/wiki/Even%E2%80%93odd_rule]
#[derive(Debug,PartialEq,Copy,Clone)]
pub enum FillingRule {
    NonZero,
    EvenOdd,
}
impl Default for FillingRule {
    fn default() -> FillingRule {
        FillingRule::NonZero
    }
}

/// Path Status
#[derive(Debug,PartialEq,Copy,Clone)]
pub enum PathStatus {
    Initial,
    Closed,
    MoveTo,
    LineTo
}
impl Default for PathStatus {
    fn default() -> PathStatus {
        PathStatus::Initial
    }
}

/// Rasterizer Anti-Alias using Scanline
#[derive(Debug, Default)]
pub struct RasterizerScanlineAA {
    /// Clipping Region
    pub clipper: Clip,
    /// Collection of Rasterizing Cells
    pub outline: RasterizerCell,
    /// Status of Path
    pub status: PathStatus,
    /// Current x position
    pub x0: i64,
    /// Current y position
    pub y0: i64,
    /// Current y row being worked on, for output
    scan_y: i64,
    /// Filling Rule for Polygons
    filling_rule: FillingRule,
    /// Gamma Corection Values
    gamma: Vec<u64>,
}

impl Rasterize for RasterizerScanlineAA {
    /// Reset Rasterizer
    ///
    /// Reset the RasterizerCell and set PathStatus to Initial
    fn reset(&mut self) {
        self.outline.reset();
        self.status = PathStatus::Initial;
    }
    /// Add a Path
    ///
    /// Walks the path from the VertexSource and rasterizes it
    fn add_path<VS: VertexSource>(&mut self, path: &VS) {
        //path.rewind();
        if ! self.outline.sorted_y.is_empty() {
            self.reset();
        }
        for seg in path.xconvert() {
            eprintln!("ADD_PATH: {:.6} {:.6}", seg.x, seg.y);
            match seg.cmd {
                PathCommand::LineTo => self.line_to_d(seg.x, seg.y),
                PathCommand::MoveTo => self.move_to_d(seg.x, seg.y),
                PathCommand::Close =>  self.close_polygon(),
                PathCommand::Stop => unimplemented!("stop encountered"),
            }
            //eprintln!("ADD_PATH: {} {} {:?} DONE", seg.x, seg.y, seg.cmd);
        }
    }

    /// Rewind the Scanline
    ///
    /// Close active polygon, sort the Rasterizer Cells, set the
    /// scan_y value to the minimum y value and return if any cells
    /// are present
    fn rewind_scanlines(&mut self) -> bool {
        self.close_polygon();
        self.outline.sort_cells();
        if self.outline.total_cells() == 0 {
            false
        } else {
            self.scan_y = self.outline.min_y;
            true
        }
    }

    /// Sweep the Scanline
    ///
    /// For individual y rows adding any to the input Scanline
    ///
    /// Returns true if data exists in the input Scanline
    fn sweep_scanline(&mut self, sl: &mut ScanlineU8) -> bool {
        loop {
            eprintln!("SWEEP SCANLINES: Y: {}", self.scan_y);
            if self.scan_y < 0 {
                self.scan_y += 1;
                continue;
            }
            if self.scan_y > self.outline.max_y {
                return false;
            }
            sl.reset_spans();
            let mut num_cells = self.outline.scanline_num_cells( self.scan_y );
            let cells = self.outline.scanline_cells( self.scan_y );

            let mut cover = 0;

            let mut iter = cells.iter();
            //eprintln!("SWEEP SCANLINES: ADDING ITER: {:?} N {}", iter, num_cells);

            if let Some(mut cur_cell) = iter.next() {
                while num_cells > 0 {
                    //eprintln!("SWEEP SCANLINES: ITER: {:?} N {}", iter, num_cells);
                    //let cur_cell = iter.next().unwrap();
                    //num_cells -= 1;

                    let mut x = cur_cell.x;
                    let mut area = cur_cell.area;

                    cover  += cur_cell.cover;
                    //eprintln!("SWEEP SCANLINES: {:?} outside cover {} area {}", cur_cell, cover, area);
                    num_cells -= 1;
                    //eprintln!("SWEEP SCANLINES: N(A): {}", num_cells); 
                    //accumulate all cells with the same X
                    while num_cells > 0 {
                        cur_cell = iter.next().unwrap();
                        //eprintln!("SWEEP SCANLINES: {:?} inside cover {} area {}", cur_cell, cover, area);
                        if cur_cell.x != x {
                            break;
                        }
                        area += cur_cell.area;
                        cover += cur_cell.cover;
                        num_cells -= 1;
                        //eprintln!("SWEEP SCANLINES: N(B): {}", num_cells); 
                    }
                    //eprintln!("SWEEP SCANLINES: {:?} DONE cover {} area {}", cur_cell, cover, area);
                    //eprintln!("SWEEP SCANLINES: ADDING CHECK AREA: {} NUM_CELLS {} x,y {} {}", area, num_cells, x, self.scan_y);
                    if area != 0 {
                        eprintln!("SWEEP SCANLINES: ADDING CELL: x {} y {} area {} cover {}", x,self.scan_y, area, cover);
                        let alpha = self.calculate_alpha((cover << (POLY_SUBPIXEL_SHIFT + 1)) - area);
                        if alpha > 0 {
                            sl.add_cell(x, alpha);
                        }
                        x += 1;
                    }
                    if num_cells > 0 && cur_cell.x > x {
                        let alpha = self.calculate_alpha(cover << (POLY_SUBPIXEL_SHIFT + 1));
                        eprintln!("SWEEP SCANLINES: ADDING SPAN: {} -> {} Y: {} area {} cover {}", x, cur_cell.x, self.scan_y, area, cover);
                        if alpha > 0 {
                            sl.add_span(x, cur_cell.x - x, alpha);
                        }
                    }
                }
            }
            if sl.num_spans() != 0 {
                break;
            }
            self.scan_y += 1;
            eprintln!("SWEEP SCANLINES:  ---------------------");
        }
        sl.finalize(self.scan_y);
        self.scan_y += 1;
        true
    }
    /// Return minimum x value from the RasterizerCell
    fn min_x(&self) -> i64 {
        self.outline.min_x
    }
    /// Return maximum x value from the RasterizerCell
    fn max_x(&self) -> i64 {
        self.outline.max_x
    }
}

impl RasterizerScanlineAA {
    /// Create a new RasterizerScanlineAA 
    pub fn new() -> Self {
        Self { clipper: Clip::new(), status: PathStatus::Initial,
               outline: RasterizerCell::new(),
               x0: 0, y0: 0, scan_y: 0,
               filling_rule: FillingRule::NonZero,
               gamma: (0..256).collect(),
        }
    }
    /// Set the gamma function
    ///
    /// Values are set as:
    ///```ignore
    ///      gamma = gfunc( v / mask ) * mask
    ///```
    /// where v = 0 to 255
    pub fn gamma<F>(&mut self, gfunc: F)
        where F: Fn(f64) -> f64
    {
        let aa_shift  = 8;
        let aa_scale  = 1 << aa_shift;
        let aa_mask   = f64::from(aa_scale - 1);

        self.gamma = (0..256)
            .map(|i| gfunc(f64::from(i) / aa_mask ))
            .map(|v| (v * aa_mask).round() as u64)
            .collect();
        for i in 0..self.gamma.len() {
            eprintln!("GAMMA: {} {}", i, self.gamma[i]);
        }
    }
    /// Create a new RasterizerScanlineAA with a gamma function
    ///
    /// See gamma() function for description
    ///
    pub fn new_with_gamma<F>(gfunc: F) -> Self
        where F: Fn(f64) -> f64
    {
        let mut new = Self::new();
        new.gamma( gfunc );
        new
    }
    /// Set Clip Box
    pub fn clip_box(&mut self, x1: f64, y1: f64, x2: f64, y2: f64) {
        self.clipper.clip_box(RasConvInt::upscale(x1),
                              RasConvInt::upscale(y1),
                              RasConvInt::upscale(x2),
                              RasConvInt::upscale(y2));
    }
    /// Move to point (x,y)
    ///
    /// Sets point as the initial point 
    pub fn move_to_d(&mut self, x: f64, y: f64) {
        self.x0 = RasConvInt::upscale( x );
        self.y0 = RasConvInt::upscale( y );
        self.clipper.move_to(self.x0,self.y0);
        self.status = PathStatus::MoveTo;
    }
    /// Draw line from previous point to point (x,y)
    pub fn line_to_d(&mut self, x: f64, y: f64) {
        let x = RasConvInt::upscale( x );
        let y = RasConvInt::upscale( y );
        self.clipper.line_to(&mut self.outline, x,y);
        self.status = PathStatus::LineTo;
    }
    /// Close the current polygon
    ///
    /// Draw a line from current point to initial "move to" point
    pub fn close_polygon(&mut self) {
        eprintln!("CLOSE POLYGON?");
        if self.status == PathStatus::LineTo {
            eprintln!("CLOSE POLYGON: CLOSED {} {}",self.x0>>8, self.y0>>8);
            self.clipper.line_to(&mut self.outline, self.x0, self.y0);
            self.status = PathStatus::Closed;
        }
    }
    /// Calculate alpha term based on area
    ///
    /// 
    pub fn calculate_alpha(&self, area: i64) -> u64 {
        let aa_shift  = 8;
        let aa_scale  = 1 << aa_shift;
        let aa_scale2 = aa_scale * 2;
        let aa_mask   = aa_scale  - 1;
        let aa_mask2  = aa_scale2 - 1;

        let mut cover = area >> (POLY_SUBPIXEL_SHIFT*2 + 1 - aa_shift);
        cover = cover.abs();
        if self.filling_rule == FillingRule::EvenOdd {
            cover *= aa_mask2;
            if cover > aa_scale {
                cover = aa_scale2 - cover;
            }
        }
        cover = max(0, min(cover, aa_mask));
        self.gamma[cover as usize]
    }
}

pub struct RasterizerOutline<'a,T> where T: PixfmtFunc + Pixel , T: 'a {
    pub ren: &'a mut RendererPrimatives<'a,T>,
    pub start_x: i64,
    pub start_y: i64,
    pub vertices: usize,
}
impl<'a,T> RasterizerOutline<'a,T> where T: PixfmtFunc + Pixel {
    pub fn with_primative(ren: &'a mut RendererPrimatives<'a,T>) -> Self {
        Self { start_x: 0, start_y: 0, vertices: 0, ren }
    }
    pub fn add_path<VS: VertexSource>(&mut self, path: &VS) {
        for v in path.xconvert().iter() {
            match v.cmd {
                PathCommand::MoveTo => self.move_to_d(v.x, v.y),
                PathCommand::LineTo => self.line_to_d(v.x, v.y),
                PathCommand::Close => self.close(),
                PathCommand::Stop => unimplemented!("stop encountered"),
            }
        }
    }
    pub fn close(&mut self) {
        if self.vertices > 2 {
            let (x,y) = (self.start_x, self.start_y);
            self.line_to( x, y );
        }
        self.vertices = 0;
    }
    pub fn move_to_d(&mut self, x: f64, y: f64) {
        eprintln!("DDA MOVED {:.6} {:.6}", x, y);
        let x = self.ren.coord(x);
        let y = self.ren.coord(y);
        self.move_to( x, y );
    }
    pub fn line_to_d(&mut self, x: f64, y: f64) {
        eprintln!("DDA LINED: {:.6} {:.6}", x, y);
        let x = self.ren.coord(x);
        let y = self.ren.coord(y);
        eprintln!("DDA LINED: {} {}", x, y);
        self.line_to( x, y );
    }
    pub fn move_to(&mut self, x: i64, y: i64) {
        self.vertices = 1;
        self.start_x = x;
        self.start_y = y;
        self.ren.move_to(x, y);
    }
    pub fn line_to(&mut self, x: i64, y: i64) {
        self.vertices += 1;
        self.ren.line_to(x, y);
    }
}
