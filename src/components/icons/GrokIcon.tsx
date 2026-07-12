import type { CSSProperties } from 'react';
import grokIcon from '../../assets/icons/grok.svg';

type GrokIconProps = {
  className?: string;
  size?: number;
  style?: CSSProperties;
};

/** Grok CLI 品牌图标：暗色圆环 + 轨道 G 形标记 */
export function GrokIcon({
  className = 'nav-item-icon',
  size,
  style,
}: GrokIconProps) {
  const mergedStyle: CSSProperties | undefined =
    typeof size === 'number'
      ? {
          width: size,
          height: size,
          ...style,
        }
      : style;

  return (
    <img
      src={grokIcon}
      className={className}
      style={mergedStyle}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
