const ParlerIcon = ({
  width,
  height,
  className,
}: {
  width?: number | string;
  height?: number | string;
  className?: string;
}) => (
  <svg
    width={width || 24}
    height={height || 24}
    viewBox="0 0 310 270"
    className={className}
    xmlns="http://www.w3.org/2000/svg"
  >
    <path
      d="M30 10 C13 10 0 23 0 40 L0 180 C0 197 13 210 30 210 L50 210 L50 260 L110 210 L280 210 C297 210 310 197 310 180 L310 40 C310 23 297 10 280 10 Z"
      fill="currentColor"
      stroke="currentColor"
      strokeWidth="8"
      strokeLinejoin="round"
    />
    <circle cx="100" cy="115" r="20" fill="white" />
    <circle cx="155" cy="115" r="20" fill="white" />
    <circle cx="210" cy="115" r="20" fill="white" />
  </svg>
);

export default ParlerIcon;
