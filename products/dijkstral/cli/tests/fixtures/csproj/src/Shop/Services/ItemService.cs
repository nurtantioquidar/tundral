using Shop.Core;

namespace Shop.Services
{
    public class ItemService : IItemService
    {
        public Item Find(string name) => new Item { Name = name };
    }

    public class Wizard : Page
    {
    }
}
