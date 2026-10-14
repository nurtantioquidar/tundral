export interface Post {
  slug: string
  title: string
  date: string
}

export function sortByDate(posts: Post[]): Post[] {
  return [...posts].sort((a, b) => b.date.localeCompare(a.date))
}

export function readingTime(text: string) {
  return Math.ceil(text.split(/\s+/).length / 200)
}

export async function getPosts(): Promise<Post[]> {
  return sortByDate([{ slug: 'hello', title: 'Hello', date: '2026-01-01' }])
}
