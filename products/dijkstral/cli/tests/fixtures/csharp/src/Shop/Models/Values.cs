namespace Shop.Models;

public readonly struct Point
{
    public Point(int x, int y)
    {
        X = x;
        Y = y;
    }

    public int X { get; }
    public int Y { get; }

    public double Length() => System.Math.Sqrt(X * X + Y * Y);
}

public record Money(decimal Amount, string Currency)
{
    public Money Add(Money other) => this with { Amount = Amount + other.Amount };
}

public record struct Range(int Start, int End);

public delegate void UserChanged(User user);
