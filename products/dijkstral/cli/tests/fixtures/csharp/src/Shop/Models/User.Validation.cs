namespace Shop.Models
{
    public partial class User
    {
        public bool IsValid()
        {
            return !string.IsNullOrWhiteSpace(GetName()) && Validate(Name);
        }

        private static bool Validate(string value) => value.Length < 200;
    }
}
