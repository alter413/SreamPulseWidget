using System.Drawing;
using Wf = System.Windows.Forms;

namespace SteamPulse;

internal sealed class DarkTrayRenderer : Wf.ToolStripProfessionalRenderer
{
    private static readonly Color Surface = Color.FromArgb(31, 34, 40);
    private static readonly Color Line = Color.FromArgb(63, 70, 82);
    private static readonly Color Hover = Color.FromArgb(48, 59, 76);
    private static readonly Color Ink = Color.FromArgb(241, 244, 250);
    private static readonly Color Muted = Color.FromArgb(154, 166, 185);

    public DarkTrayRenderer() : base(new Palette()) => RoundedEdges = false;

    protected override void OnRenderItemText(Wf.ToolStripItemTextRenderEventArgs e)
    {
        e.TextColor = e.Item.Enabled ? Ink : Muted;
        base.OnRenderItemText(e);
    }

    protected override void OnRenderArrow(Wf.ToolStripArrowRenderEventArgs e)
    {
        e.ArrowColor = e.Item?.Enabled == false ? Muted : Ink;
        base.OnRenderArrow(e);
    }

    private sealed class Palette : Wf.ProfessionalColorTable
    {
        public override Color ToolStripDropDownBackground => Surface;
        public override Color MenuBorder => Line;
        public override Color MenuItemBorder => Hover;
        public override Color MenuItemSelected => Hover;
        public override Color MenuItemSelectedGradientBegin => Hover;
        public override Color MenuItemSelectedGradientEnd => Hover;
        public override Color ImageMarginGradientBegin => Surface;
        public override Color ImageMarginGradientMiddle => Surface;
        public override Color ImageMarginGradientEnd => Surface;
        public override Color SeparatorDark => Line;
        public override Color SeparatorLight => Surface;
    }
}
