using System;
using System.Collections.Generic;

namespace Shop.Models
{
    public enum Status
    {
        Active,
        Suspended = 2,
        Deleted,
    }

    /// <summary>
    /// A registered customer.
    /// </summary>
    [Serializable]
    public partial class User : Entity, IComparable<User>
    {
        private readonly List<Address> _addresses = new List<Address>();

        public string Name { get; set; }

        public Status Status { get; private set; } = Status.Active;

        public event EventHandler? Renamed;

        public User(string id, string name) : base(id)
        {
            Name = name;
        }

        public User(string id) : this(id, "anonymous")
        {
        }

        public string GetName()
        {
            return Name;
        }

        public void AddAddress(Address address)
        {
            _addresses.Add(address);
            Touch();
        }

        public override string Describe() => $"User {GetName()}";

        public int CompareTo(User? other) => string.Compare(Name, other?.Name, StringComparison.Ordinal);

        private void Touch()
        {
            // WHY: listeners rely on a rename event even when the name is unchanged.
            Renamed?.Invoke(this, EventArgs.Empty);
        }

        public static User Of(string name) => new User(Guid.NewGuid().ToString(), name);

        public class Address
        {
            public string Street { get; init; } = "";

            public string Format() => Street.Trim();
        }
    }
}
