import type { CSSProperties, ImgHTMLAttributes } from 'react';
// Official Kimi Code product mark (VS Code extension storefront icon from MoonshotAI).
// Source: marketplace Microsoft.VisualStudio.Services.Icons.Default /
// github.com/MoonshotAI/kimi-code apps/vscode/resources/kimi-icon-storefront.png
import kimiCodeIcon from '../../assets/icons/kimi-code.png';

interface KimiIconProps extends Omit<ImgHTMLAttributes<HTMLImageElement>, 'src' | 'alt'> {
  size?: number;
  style?: CSSProperties;
  className?: string;
}

export function KimiIcon({
  size = 20,
  style,
  className = 'nav-item-icon',
  ...props
}: KimiIconProps) {
  return (
    <img
      src={kimiCodeIcon}
      alt=""
      className={className}
      width={size}
      height={size}
      style={{
        width: size,
        height: size,
        display: 'inline-block',
        objectFit: 'contain',
        borderRadius: '22%',
        ...style,
      }}
      aria-hidden="true"
      draggable={false}
      {...props}
    />
  );
}
