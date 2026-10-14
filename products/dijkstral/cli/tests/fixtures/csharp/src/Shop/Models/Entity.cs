using System;

namespace Shop.Models;

/// <summary>
/// Something with a stable identity.
/// </summary>
public interface IEntity<TKey>
{
    TKey Id { get; }

    string Describe();
}

/// <summary>Base class for every persisted model.</summary>
public abstract class Entity : IEntity<string>
{
    // NOTE: ids are assigned by the repository, never by callers.
    public string Id { get; protected set; } = string.Empty;

    public DateTime CreatedAt { get; } = DateTime.UtcNow;

    protected Entity(string id)
    {
        Id = id;
    }

    public abstract string Describe();

    public virtual bool IsNew() => string.IsNullOrEmpty(Id);
}
