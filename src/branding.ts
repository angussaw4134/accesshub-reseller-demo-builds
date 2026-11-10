export type ResellerBranding = {
  slug: string;
  name: string;
  tagline: string;
  loginTitle: string;
  loginMessage: string;
  accent: string;
  background: string;
  supportSnapchat: string;
  logoDataUrl: string;
  version: number;
  updatedAt: string | null;
};

export function readBranding(value: unknown): ResellerBranding | null {
  if (!value || typeof value !== 'object') return null;
  const brand = value as ResellerBranding;
  const text = (input: unknown, max: number) => typeof input === 'string' && input.length <= max && !/[<>\u0000-\u0008\u000b\u000c\u000e-\u001f]/.test(input);
  if (!text(brand.name, 100) || !brand.name.trim() || !text(brand.tagline, 160) ||
      !text(brand.loginTitle, 100) || !text(brand.loginMessage, 500) ||
      !/^#[a-f0-9]{6}$/i.test(brand.accent) || !/^#[a-f0-9]{6}$/i.test(brand.background) ||
      !/^[a-z0-9._-]{2,32}$/.test(brand.supportSnapchat) ||
      typeof brand.logoDataUrl !== 'string' || brand.logoDataUrl.length > 205000 ||
      (brand.logoDataUrl && !/^data:image\/(png|jpeg|webp);base64,[A-Za-z0-9+/]+={0,2}$/.test(brand.logoDataUrl))) return null;
  return brand;
}

export function readableColor(background: string, preferred = '#ffffff'): string {
  const luminance = (hex: string) => {
    const [r, g, b] = [1, 3, 5].map(index => parseInt(hex.slice(index, index + 2), 16) / 255)
      .map(value => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
    return 0.2126 * r + 0.7152 * g + 0.0722 * b;
  };
  const base = luminance(background), candidate = luminance(preferred);
  const contrast = (Math.max(base, candidate) + 0.05) / (Math.min(base, candidate) + 0.05);
  return contrast >= 4.5 ? preferred : base > 0.179 ? '#000000' : '#ffffff';
}

export function accentInk(hex: string): string {
  return readableColor(hex, '#171019');
}
