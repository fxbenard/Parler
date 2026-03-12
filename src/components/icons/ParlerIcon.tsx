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
    width={width || 512}
    height={height || 512}
    viewBox="0 0 512 512"
    className={className}
    xmlns="http://www.w3.org/2000/svg"
  >
    <rect
      x="0"
      y="0"
      width="512"
      height="512"
      rx="110"
      ry="110"
      fill="#3B7DD8"
    />
    <path
      d="M120 160 C120 130 145 105 175 105 L337 105 C367 105 392 130 392 160 L392 295 C392 325 367 350 337 350 L200 350 L145 400 L145 350 C130 350 120 340 120 325 Z"
      fill="white"
    />
    <circle cx="210" cy="228" r="22" fill="#3B7DD8" />
    <circle cx="270" cy="228" r="22" fill="#3B7DD8" />
    <circle cx="330" cy="228" r="22" fill="#3B7DD8" />
  </svg>
);

export default ParlerIcon;
