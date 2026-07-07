import { CSSProperties } from 'react';
import traeIcon from '../../assets/icons/trae.png';
import traeSoloIcon from '../../assets/icons/trae-solo.png';
import traeCnIcon from '../../assets/icons/trae-cn.png';
import traeSoloCnIcon from '../../assets/icons/trae-solo-cn.png';

type TraeIconProps = {
  className?: string;
  style?: CSSProperties;
};

type TraeImageIconProps = TraeIconProps & {
  src: string;
};

function TraeImageIcon({ src, className = 'nav-item-icon', style }: TraeImageIconProps) {
  return (
    <img
      className={className}
      style={style}
      src={src}
      alt=""
      aria-hidden="true"
      draggable={false}
    />
  );
}

export function TraeIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeIcon} />;
}

export function TraeSoloIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeSoloIcon} />;
}

export function TraeCnIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeCnIcon} />;
}

export function TraeSoloCnIcon(props: TraeIconProps) {
  return <TraeImageIcon {...props} src={traeSoloCnIcon} />;
}
