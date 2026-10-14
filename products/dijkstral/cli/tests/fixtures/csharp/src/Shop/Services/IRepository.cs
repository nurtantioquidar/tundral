using System.Collections.Generic;
using Shop.Models;

namespace Shop.Services
{
    public interface IRepository<T> where T : Entity
    {
        void Save(T item);

        T? Find(string id);

        IEnumerable<T> All();
    }

    public class InMemoryRepository<T> : IRepository<T> where T : Entity
    {
        private readonly Dictionary<string, T> _items = new();

        public void Save(T item) => _items[item.Id] = item;

        public T? Find(string id) => _items.TryGetValue(id, out var item) ? item : null;

        public IEnumerable<T> All() => _items.Values;
    }
}
