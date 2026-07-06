import { CSSProperties } from 'react';
import qoderIcon from '../../assets/icons/qoder.png';

type QoderworkCnIconProps = {
  className?: string;
  style?: CSSProperties;
};

export function QoderworkCnIcon({ className = 'nav-item-icon', style }: QoderworkCnIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={qoderIcon}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}
