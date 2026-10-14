import { useState } from 'react'

export default function Like({ initial }: { initial: number }) {
  const [n, setN] = useState(initial)
  return <button onClick={() => setN(bump(n))}>{n}</button>
}

export function bump(n: number) {
  return n + 1
}
