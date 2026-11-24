$ErrorActionPreference = 'Stop'
Add-Type -ReferencedAssemblies System.Drawing,System.Windows.Forms -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public class WorkshopSnip : Form {
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr value);
    readonly Bitmap desktop;
    Point start;
    bool dragging;
    Rectangle selection;
    WorkshopSnip(Bitmap desktop) {
        this.desktop = desktop;
        Text = "Select a screenshot area";
        AutoScaleMode = AutoScaleMode.None;
        FormBorderStyle = FormBorderStyle.None;
        StartPosition = FormStartPosition.Manual;
        Bounds = SystemInformation.VirtualScreen;
        TopMost = true;
        ShowInTaskbar = false;
        DoubleBuffered = true;
        BackColor = Color.Black;
        Cursor = Cursors.Cross;
        AccessibleDescription = "Drag to send a screenshot. Press Escape to cancel.";
    }
    protected override void OnShown(EventArgs e) { base.OnShown(e); Activate(); }
    protected override bool ProcessCmdKey(ref Message msg, Keys key) {
        if (key == Keys.Escape) { DialogResult = DialogResult.Cancel; return true; }
        return base.ProcessCmdKey(ref msg, key);
    }
    protected override void OnMouseDown(MouseEventArgs e) {
        if (e.Button == MouseButtons.Right) { DialogResult = DialogResult.Cancel; return; }
        if (e.Button != MouseButtons.Left) return;
        start = e.Location; dragging = Capture = true;
    }
    Rectangle Area(Point end) {
        return Rectangle.Intersect(ClientRectangle, Rectangle.FromLTRB(Math.Min(start.X,end.X),Math.Min(start.Y,end.Y),Math.Max(start.X,end.X),Math.Max(start.Y,end.Y)));
    }
    protected override void OnMouseMove(MouseEventArgs e) {
        if (dragging) { selection = Area(e.Location); Invalidate(); }
    }
    protected override void OnMouseUp(MouseEventArgs e) {
        if (!dragging || e.Button != MouseButtons.Left) return;
        dragging = Capture = false;
        selection = Area(e.Location);
        if (selection.Width > 1 && selection.Height > 1) {
            DialogResult = DialogResult.OK;
        } else Invalidate();
    }
    protected override void OnPaint(PaintEventArgs e) {
        e.Graphics.DrawImageUnscaled(desktop,0,0);
        using (var shade = new SolidBrush(Color.FromArgb(82,0,0,0))) e.Graphics.FillRectangle(shade,ClientRectangle);
        if (dragging && selection.Width > 0 && selection.Height > 0) {
            e.Graphics.DrawImage(desktop,selection,selection,GraphicsUnit.Pixel);
            e.Graphics.DrawRectangle(Pens.White, selection);
            return;
        }
        Rectangle screen = Screen.FromPoint(Cursor.Position).WorkingArea;
        Point label = PointToClient(new Point(screen.Left+24,screen.Top+24));
        using (Font font = new Font("Segoe UI",16))
            e.Graphics.DrawString("Drag to send a screenshot   ·   Esc to cancel", font, Brushes.White,label);
    }
    public static void Run() {
        SetThreadDpiAwarenessContext(new IntPtr(-4));
        Rectangle screen = SystemInformation.VirtualScreen;
        using (var desktop = new Bitmap(screen.Width, screen.Height, PixelFormat.Format24bppRgb)) {
            using (var g = Graphics.FromImage(desktop)) g.CopyFromScreen(screen.Location,Point.Empty,screen.Size);
            Rectangle area;
            using (var overlay = new WorkshopSnip(desktop)) {
                if (overlay.ShowDialog() != DialogResult.OK) return;
                area = overlay.selection;
            }
            using (var capture = desktop.Clone(area, PixelFormat.Format24bppRgb)) Save(capture);
        }
    }
    static void Save(Bitmap capture) {
            capture.Save("capture.png", ImageFormat.Png);
            double scale = Math.Min(1, Math.Min(480.0/capture.Width,320.0/capture.Height));
            using (var thumb = new Bitmap(Math.Max(1,(int)(capture.Width*scale)),Math.Max(1,(int)(capture.Height*scale)))) {
                using (var g = Graphics.FromImage(thumb)) {
                    g.InterpolationMode = InterpolationMode.HighQualityBicubic;
                    g.DrawImage(capture,0,0,thumb.Width,thumb.Height);
                }
                thumb.Save("preview.png",ImageFormat.Png);
            }
    }
}
'@
[WorkshopSnip]::Run()
